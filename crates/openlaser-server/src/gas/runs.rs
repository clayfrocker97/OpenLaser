// SPDX-License-Identifier: GPL-3.0-or-later

//! The run history: one record per job run, with what it spent. A run that
//! is stopped and later resumed keeps one record, updated as it goes on.

use super::Consumption;
use openlaser_library::Id;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How a run ended.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// Every pass ran.
    Done,
    /// Stopped, or failed, before the end.
    Stopped,
}

/// One run of a job and what it spent.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// Stable identity.
    pub id: String,
    /// The authoring document it ran from, as `job-…`, `draft-…` or `part-…`.
    pub key: String,
    /// The saved job, if it was one.
    pub job: Option<Id>,
    /// The job's name when it ran.
    pub name: String,
    /// When it started, in seconds since the epoch.
    pub started: u64,
    /// When it last ended.
    pub finished: u64,
    /// Done or stopped.
    pub outcome: RunOutcome,
    /// The share of the job's cut length that ran, 0 to 1.
    pub fraction: f64,
    /// Laser time, gas and cost of what ran, priced when it ended.
    pub consumption: Consumption,
    /// The currency symbol it was priced in.
    pub currency: String,
}

/// The most runs kept; the oldest go first.
pub const MAX_RUNS: usize = 2000;

const FILE: &str = "gas-runs.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    runs: Vec<RunRecord>,
}

pub(super) struct Runs {
    path: PathBuf,
    records: Vec<RunRecord>,
    limit: usize,
    /// Set when the saved file could not be read; it is then never overwritten.
    error: Option<String>,
}

impl Runs {
    pub(super) fn open(root: &Path) -> Self {
        let path = root.join(FILE);
        let (runs, error) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Saved>(&bytes) {
                Ok(saved) if saved.version == 1 => (saved.runs, None),
                Ok(saved) => (Vec::new(), Some(format!("unsupported version {}", saved.version))),
                Err(e) => (Vec::new(), Some(e.to_string())),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
        if let Some(error) = &error {
            tracing::warn!(%error, path = %path.display(), "gas run history unreadable");
        }
        Self {
            path,
            records: runs,
            limit: MAX_RUNS,
            error: error.map(|e| {
                format!("The run history could not be read ({e}); new runs are not saved.")
            }),
        }
    }

    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(super) fn matching(&self, key: &str, job: Option<&str>) -> Vec<RunRecord> {
        self.records
            .iter()
            .rev()
            .filter(|r| {
                r.key == key
                    || job.is_some_and(|job| r.job.as_ref().is_some_and(|j| j.as_str() == job))
            })
            .cloned()
            .collect()
    }

    pub(super) fn upsert(&mut self, record: RunRecord) -> crate::Result<()> {
        if let Some(error) = &self.error {
            return Err(crate::Error::Refused(error.clone()));
        }
        let mut next = self.records.clone();
        if let Some(existing) = next.iter_mut().find(|r| r.id == record.id) {
            *existing = record;
        } else {
            next.push(record);
        }
        let excess = next.len().saturating_sub(self.limit);
        next.drain(..excess);
        let bytes = serde_json::to_vec(&Saved { version: 1, runs: next.clone() })
            .map_err(|e| crate::Error::Refused(e.to_string()))?;
        openlaser_library::atomic_write(&self.path, &bytes)?;
        self.records = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, key: &str) -> RunRecord {
        RunRecord {
            id: id.into(),
            key: key.into(),
            job: None,
            name: "Plate".into(),
            started: 1,
            finished: 2,
            outcome: RunOutcome::Stopped,
            fraction: 0.4,
            consumption: Consumption::default(),
            currency: "$".into(),
        }
    }

    #[test]
    fn a_resumed_run_updates_its_record_and_history_is_bounded() {
        let root = std::env::temp_dir().join(format!("openlaser-gas-runs-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&root));
        std::fs::create_dir_all(&root).unwrap();
        let mut runs = Runs::open(&root);
        runs.upsert(record("a", "job-1")).unwrap();
        runs.upsert(record("b", "draft-2")).unwrap();
        let mut done = record("a", "job-1");
        done.outcome = RunOutcome::Done;
        done.fraction = 1.;
        runs.upsert(done.clone()).unwrap();
        let reopened = Runs::open(&root);
        assert_eq!(reopened.matching("job-1", None), vec![done]);
        assert_eq!(reopened.matching("other", None).len(), 0);
        runs.limit = 5;
        for i in 0..5 {
            runs.upsert(record(&format!("r{i}"), "job-1")).unwrap();
        }
        assert_eq!(runs.records.len(), 5);
        assert_eq!(runs.records[0].id, "r0", "the oldest runs are dropped first");

        std::fs::write(root.join(FILE), b"not json").unwrap();
        let mut damaged = Runs::open(&root);
        assert!(damaged.error().is_some());
        assert!(damaged.upsert(record("c", "job-1")).is_err());
        assert_eq!(std::fs::read(root.join(FILE)).unwrap(), b"not json");
    }
}
