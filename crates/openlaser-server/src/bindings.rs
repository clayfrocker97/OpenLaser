// SPDX-License-Identifier: GPL-3.0-or-later

//! The vendor's machine backup a machine is configured from, and how the
//! controller task takes its bindings.

use crate::document::{BankView, CapabilitiesView, FileView, GasRouteView, RecipeSummary};
use openlaser_controller::alarms::Rule;
use openlaser_controller::bindings::{Bindings, HomeOutputs};
use openlaser_core::LaserMode;
use openlaser_xml::bindings as vendor;
use openlaser_xml::{Bundle, Document, Kind, layer_file};
use std::path::{Path, PathBuf};

/// The machine backup: the path in `machine.toml`, replaced by the one
/// imported into the data directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Files {
    /// The vendor's backup, which holds every parameter.
    pub backup: Option<PathBuf>,
}

impl Files {
    /// The configured backup, replaced by the one kept in `dir`.
    pub fn with_dir(mut self, dir: &Path) -> Result<Self, String> {
        if let Some(imported) = crate::machine_files::imported(dir)? {
            self.backup = Some(imported);
        }
        Ok(self)
    }
}

/// Checks that `bytes` are a machine backup that binds for `mode`, the
/// file's own saved mode when none is given, so an incomplete file never
/// replaces a good one. Any valid scale proves the groups are there.
pub fn check(bytes: &[u8], mode: Option<LaserMode>) -> Result<(), openlaser_xml::Error> {
    let mut bundle = Bundle::default();
    bundle.insert(Document::parse(Kind::Backup, bytes)?);
    let mode = mode.or_else(|| saved_mode(&bundle)).unwrap_or(LaserMode::Fiber);
    vendor::bind(&bundle, mode, 1000).map(drop)
}

/// What the backup reads as.
pub struct Loaded {
    /// The parameter groups.
    pub bundle: Bundle,
    /// The file, for the settings page.
    pub file: FileView,
}

/// Reads the configured backup.
pub fn load(files: &Files) -> Result<Loaded, String> {
    let path = files.backup.as_ref().ok_or("no machine backup is configured")?;
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bundle = Bundle::default();
    bundle.insert(
        Document::parse(Kind::Backup, &bytes).map_err(|e| format!("{}: {e}", path.display()))?,
    );
    let hash = openlaser_library::sha256(&bytes);
    let file = FileView {
        name: crate::machine_files::display_name(path, &hash)?,
        bytes: bytes.len() as u64,
        sha256: hash,
    };
    Ok(Loaded { bundle, file })
}

/// The layer banks the files hold, for choosing one as a recipe.
#[must_use]
pub fn banks(bundle: &Bundle) -> Vec<BankView> {
    let mut banks = Vec::new();
    for laser in [LaserMode::Fiber, LaserMode::Co2] {
        for bank in 1..=11 {
            let Ok(attributes) = bundle.group(&layer_file::group(laser, bank), "GP") else {
                continue;
            };
            banks.push(BankView {
                laser,
                bank,
                name: attributes.get("LayerFileName").cloned().unwrap_or_default(),
                disabled: attributes.get("NoManu").is_some_and(|v| v == "1"),
                summary: RecipeSummary::of(attributes),
            });
        }
    }
    banks
}

/// The gas selections the machine can make: those the backup gives a
/// valve, in the controller's order.
#[must_use]
pub fn gases(bundle: &Bundle) -> Vec<u8> {
    let Ok(ports) = bundle.group("GasParam", "MGP") else { return Vec::new() };
    ["LowAir", "LowO2", "LowN2", "HighAir", "HighO2", "HighN2"]
        .into_iter()
        .enumerate()
        .filter(|(_, key)| ports.get(*key).is_some_and(|port| port.trim() != "0"))
        .filter_map(|(i, _)| u8::try_from(i).ok())
        .collect()
}

/// What `mode`'s process can do on this machine, or nothing when its
/// hardware does not bind. `manual_height` is the operator's CO2 setup,
/// which runs without the head.
#[must_use]
pub fn capabilities(
    bundle: &Bundle,
    mode: LaserMode,
    manual_height: bool,
) -> Option<CapabilitiesView> {
    let hardware = openlaser_xml::recipe::hardware(bundle, mode).ok()?;
    let text = |group: &str, tag: &str, key: &str| {
        bundle.group(group, tag).ok().and_then(|a| a.get(key)).map(|v| v.trim().to_owned())
    };
    let gases = crate::recipes::GAS
        .iter()
        .enumerate()
        .map(|(i, name)| GasRouteView {
            selector: u8::try_from(i).unwrap_or(0),
            name: (*name).to_owned(),
            valve: hardware.gas_enabled && hardware.gas_ports[i].is_some(),
            pressure: hardware.gas_enabled && i < 3 && hardware.ratio_channels[i].is_some(),
        })
        .collect();
    Some(CapabilitiesView {
        laser: mode,
        height_control: hardware.z_enabled && !manual_height,
        gases,
        peak_output: hardware.peak_channel.is_some(),
        pre_pierce_batch: text("SoftParam", "GP", "PreDrillMaxNum").and_then(|v| v.parse().ok()),
        film_batch: text("ManuParam", "MC", "ClearUpFilmNum_Pre").and_then(|v| v.parse().ok()),
        short_transfer_mm: text("ManuParam", "FC", "ShortNoUpMaxLength")
            .and_then(|v| v.parse().ok()),
        gas_delays_ms: [
            hardware.gas_delay_ms,
            hardware.first_gas_delay_ms,
            hardware.change_gas_delay_ms,
        ],
    })
}

/// The controller's bindings from the vendor's.
#[must_use]
pub fn controller(vendor: &vendor::Bindings) -> Bindings {
    Bindings {
        mode: vendor.mode,
        head_enabled: vendor.head_enabled,
        shutdown: vendor.shutdown.clone(),
        home: HomeOutputs {
            z_origin_done_port: vendor.home.z_origin_done_port,
            manual_signal_port: vendor.home.manual_signal_port,
        },
        mode_switch: vendor.mode_switch.clone(),
        dual_drive: Some(vendor.dual_drive),
        rules: vendor
            .rules
            .iter()
            .map(|rule| Rule {
                id: rule.id,
                label: rule.label.clone(),
                input: rule.input,
                active_low: rule.active_low,
                run_only: rule.run_only,
                latch: rule.latch,
                gas_valve: rule.gas_valve,
            })
            .collect(),
        co2_pwm_sync_port: vendor.co2_pwm_sync_port,
    }
}

/// The laser the vendor files were last set to, when they say.
#[must_use]
pub fn saved_mode(bundle: &Bundle) -> Option<LaserMode> {
    match bundle.saved_laser_mode() {
        Ok(Some(0)) => Some(LaserMode::Fiber),
        Ok(Some(1)) => Some(LaserMode::Co2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The backup kept in the import directory replaces the configured
    /// one, and only a file that parses and binds passes the check.
    #[test]
    fn the_imported_backup_replaces_the_configured_one() {
        let dir =
            std::env::temp_dir().join(format!("openlaser-files-{}", openlaser_library::now()));
        std::fs::create_dir_all(&dir).unwrap();
        let configured = Files { backup: Some(PathBuf::from("configured/backup.xml")) };
        assert_eq!(configured.clone().with_dir(&dir).unwrap(), configured, "nothing imported yet");
        std::fs::write(dir.join("1390backup.xml"), b"<ParameterRoot/>").unwrap();
        assert_eq!(configured.with_dir(&dir).unwrap().backup, Some(dir.join("1390backup.xml")));
        assert!(matches!(check(b"<ParameterRoot/>", None), Err(openlaser_xml::Error::Missing(_))));
        assert!(check(b"<Other/>", None).is_err());
    }
}
