// SPDX-License-Identifier: GPL-3.0-or-later

//! Lossless XML settings and controller comparisons from the same write plan.

use crate::coordinator::Shared;
use crate::{Error, Result};
use openlaser_controller::session::Verified;
use openlaser_core::LaserMode;
use openlaser_xml::{Bundle, Document, Kind, initialization};
use serde::{Deserialize, Serialize};

pub(crate) fn document(bundle: &Bundle) -> Result<&Document> {
    bundle
        .document(Kind::Backup)
        .or_else(|| bundle.document(Kind::Hardware))
        .ok_or_else(|| Error::Missing("import a machine backup first".into()))
}

/// One register comparison. Values are controller words, not XML units.
#[derive(Clone, Debug, Serialize)]
pub struct Comparison {
    /// Native register address.
    pub address: u32,
    /// Bits represented by these fields.
    pub mask: u32,
    /// Expected masked controller word.
    pub expected: u32,
    /// Freshly observed masked word, when connected.
    pub actual: Option<u32>,
    /// Full XML paths determining this word.
    pub fields: Vec<String>,
}

/// An editable attribute, including attributes unknown to the projection.
#[derive(Clone, Debug, Serialize)]
pub struct Field {
    /// Full element path.
    pub path: String,
    /// Exact case-sensitive XML attribute name.
    pub name: String,
    /// Decoded original value; saving preserves all unrelated bytes.
    pub value: String,
}

/// Current XML and comparison of controller-backed settings.
#[derive(Clone, Debug, Serialize)]
pub struct View {
    /// Source filename.
    pub name: String,
    /// Hash required for edits and writes.
    pub sha256: String,
    /// Every attribute in document order.
    pub fields: Vec<Field>,
    /// Word-level comparisons, including shared packed words.
    pub comparisons: Vec<Comparison>,
    /// Why initialization or comparison cannot currently be prepared.
    pub problem: Option<String>,
    /// Whether the comparison has current controller feedback.
    pub connected: bool,
}

/// One proposed attribute edit.
#[derive(Clone, Debug, Deserialize)]
pub struct Edit {
    /// Full element path.
    pub path: String,
    /// Exact XML attribute name.
    pub name: String,
    /// New decoded value.
    pub value: String,
}

/// Versioned edits, preventing a stale tab from replacing a newer backup.
#[derive(Clone, Debug, Deserialize)]
pub struct Change {
    /// Reviewed backup hash.
    pub expected: String,
    /// Attributes to replace in place.
    pub edits: Vec<Edit>,
}

pub(crate) fn actual(verified: &Verified, address: u32) -> Option<u32> {
    if (50000..=50050).contains(&address) && address.is_multiple_of(2) {
        verified.system.get(usize::try_from((address - 50000) / 2).ok()?).copied()
    } else if (50106..=50108).contains(&address) && address.is_multiple_of(2) {
        verified.pwm.get(usize::try_from((address - 50106) / 2).ok()?).copied()
    } else if (50200..50388).contains(&address) && address.is_multiple_of(2) {
        let offset = usize::try_from(address - 50200).ok()?;
        verified.banks.get(offset / 40)?.get(offset % 40 / 2).copied()
    } else {
        None
    }
}

pub(crate) fn mismatches(
    doc: &Document,
    verified: &Verified,
    mode: LaserMode,
) -> Result<Vec<String>> {
    let plan = initialization::Plan::from_document(doc, verified.scale, mode)?;
    Ok(plan
        .checks
        .iter()
        .filter_map(|check| {
            let actual = actual(verified, check.address).map(|word| word & check.mask);
            (actual != Some(check.value)).then(|| {
                format!(
                    "{} (register {}: expected {}, read {})",
                    check.fields.join(" + "),
                    check.address,
                    check.value,
                    actual.map_or_else(|| "unavailable".into(), |v| v.to_string())
                )
            })
        })
        .collect())
}

/// Returns every field, with readback only where a projection exists.
pub async fn view(shared: &Shared) -> Result<View> {
    let c = shared.lock().await;
    let file = c
        .files
        .backup
        .as_ref()
        .ok_or_else(|| Error::Missing("import a machine backup first".into()))?;
    let bundle = c.bundle.as_ref().ok_or_else(|| Error::Missing("no machine backup".into()))?;
    let doc = document(bundle)?;
    let mut fields = Vec::new();
    for path in doc.paths() {
        for (name, value) in doc.attributes(path)? {
            fields.push(Field { path: path.into(), name: name.clone(), value: value.clone() });
        }
    }
    let state = c.machine.state();
    let measured = c
        .connected()
        .then_some(state.observed_parameters)
        .flatten()
        .filter(|_| state.feedback.as_ref().is_some_and(|f| f.age_ms <= 1000));
    let scale = measured.map_or(1000, |v| v.scale);
    let plan = initialization::Plan::from_document(doc, scale, c.mode.unwrap_or(LaserMode::Fiber));
    let (comparisons, problem) = match plan {
        Ok(plan) => (
            plan.checks
                .into_iter()
                .map(|check| Comparison {
                    address: check.address,
                    mask: check.mask,
                    expected: check.value,
                    actual: measured
                        .as_ref()
                        .and_then(|v| actual(v, check.address))
                        .map(|v| v & check.mask),
                    fields: check.fields,
                })
                .collect(),
            c.parameter_problem.clone(),
        ),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    Ok(View {
        name: file.name.clone(),
        sha256: file.sha256.clone(),
        fields,
        comparisons,
        problem,
        connected: measured.is_some(),
    })
}

/// Saves only changed spans, then follows the ordinary machine import flow.
pub async fn save(shared: &Shared, change: Change) -> Result<()> {
    if change.edits.len() > 10000 {
        return Err(Error::Request("too many settings edits".into()));
    }
    let (doc, name) = {
        let c = shared.lock().await;
        let file =
            c.files.backup.as_ref().ok_or_else(|| Error::Missing("no machine backup".into()))?;
        if file.sha256 != change.expected {
            return Err(Error::Refused("the backup changed; reload settings before saving".into()));
        }
        let bundle = c.bundle.as_ref().ok_or_else(|| Error::Missing("no machine backup".into()))?;
        (document(bundle)?.clone(), file.name.clone())
    };
    let edited = tokio::task::spawn_blocking(move || {
        let mut doc = doc;
        let mut seen = std::collections::BTreeSet::new();
        for edit in change.edits {
            if !seen.insert((edit.path.clone(), edit.name.clone())) {
                return Err(Error::Request("a setting was supplied twice".into()));
            }
            doc = doc.with_attribute(&edit.path, &edit.name, &edit.value)?;
        }
        Ok::<_, Error>(doc)
    })
    .await
    .map_err(|e| Error::Refused(e.to_string()))??;
    crate::machine::import_file_reviewed(shared, &name, edited.original(), Some(&change.expected))
        .await
}

/// Projects and applies the currently bound XML. Caller owns configuration admission.
pub(crate) async fn initialize(shared: &Shared) -> Result<()> {
    let epoch = shared.ensure_running()?;
    let (doc, mode) = {
        let mut c = shared.lock().await;
        c.accepted = None;
        c.parameter_problem = Some("applying machine settings".into());
        let bundle = c.bundle.as_ref().ok_or_else(|| Error::Missing("no machine backup".into()))?;
        (document(bundle)?.clone(), c.mode.unwrap_or(LaserMode::Fiber))
    };
    let before = shared.machine.read_parameters().await?;
    let plan = initialization::Plan::from_document(&doc, before.scale, mode)?;
    let expected = expected_parameters(&before, &plan)?;
    if shared.ensure_running()? != epoch {
        return Err(Error::Refused("initialization was cancelled".into()));
    }
    shared
        .machine
        .initialize(openlaser_controller::operations::parameters::Initialization {
            before,
            expected,
            writes: plan.writes,
            idle_outputs: plan.idle_outputs,
        })
        .await?;
    Ok(())
}

fn expected_parameters(
    before: &openlaser_controller::session::Verified,
    plan: &initialization::Plan,
) -> Result<openlaser_controller::session::Verified> {
    let mut expected = *before;
    for write in &plan.writes {
        if (50200..=50360).contains(&write.address) && write.words.len() == 14 {
            let axis = usize::try_from((write.address - 50200) / 40)
                .map_err(|e| Error::Refused(e.to_string()))?;
            expected.banks[axis].copy_from_slice(&write.words);
        }
    }
    for check in &plan.checks {
        if (50000..=50050).contains(&check.address) {
            expected.system[usize::try_from((check.address - 50000) / 2)
                .map_err(|e| Error::Refused(e.to_string()))?] = check.value;
        } else if (50106..=50108).contains(&check.address) {
            expected.pwm[usize::try_from((check.address - 50106) / 2)
                .map_err(|e| Error::Refused(e.to_string()))?] = check.value;
        }
    }
    Ok(expected)
}
