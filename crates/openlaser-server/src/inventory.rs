// SPDX-License-Identifier: GPL-3.0-or-later

//! Full sheets on the rack: how many of each size, per material, thickness
//! and laser. Nesting draws on them and a cut takes one off. Remnants stay
//! in the sheet history, which holds their shape; both appear together in
//! the library view so screens organise them in one place.

use crate::stock_store::SheetPlan;
use crate::{Coordinator, Error, Result};
use openlaser_core::LaserMode;
use openlaser_library::Id;
use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE: &str = "inventory.json";
/// No sheet side is longer than this; it rejects unit mistakes.
const MAX_SIDE_MM: f64 = 20_000.;
/// More than any rack holds; it rejects typing slips.
const MAX_QUANTITY: u32 = 10_000;
/// Sizes this close are the same sheet.
const SIZE_TOLERANCE_MM: f64 = 0.01;

/// One size of full sheet in one material, and how many are on hand.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockItem {
    /// Stable identity.
    pub id: Id,
    /// Material name, as the recipes name it.
    pub material: String,
    /// Thickness in millimetres.
    pub thickness_mm: f64,
    /// The laser its recipes are for.
    pub laser: LaserMode,
    /// Along X, in millimetres.
    pub width_mm: f64,
    /// Along Y, in millimetres.
    pub height_mm: f64,
    /// Sheets on hand; zero keeps the size listed for restocking.
    pub quantity: u32,
    /// The library folder it is kept in, beside its jobs, or the root.
    pub folder: Option<Id>,
}

/// Sheets to add: an existing entry of the same material, size and folder
/// gains the quantity, otherwise a new entry starts.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewStock {
    /// Material name, as the recipes name it.
    pub material: String,
    /// Thickness in millimetres.
    pub thickness_mm: f64,
    /// The laser its recipes are for.
    pub laser: LaserMode,
    /// Along X, in millimetres.
    pub width_mm: f64,
    /// Along Y, in millimetres.
    pub height_mm: f64,
    /// Sheets to add.
    pub quantity: u32,
    /// The folder to keep them in.
    #[serde(default)]
    pub folder: Option<Id>,
}

/// A change to one entry; absent fields stay as they are.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export, optional_fields))]
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockChange {
    /// Sheets on hand.
    pub quantity: Option<u32>,
    /// Move to this folder; `Some(None)` moves it to the root.
    #[serde(default, deserialize_with = "crate::coordinator::double_option")]
    #[allow(clippy::option_option, reason = "absent leaves alone, null clears")]
    pub folder: Option<Option<Id>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    items: Vec<StockItem>,
}

impl StockItem {
    /// Whether these sheets suit a recipe: same laser, material and thickness.
    #[must_use]
    pub fn suits(&self, laser: LaserMode, material: &str, thickness_mm: f64) -> bool {
        suits(self.laser, &self.material, self.thickness_mm, laser, material, thickness_mm)
    }

    fn validate(&self) -> Result<()> {
        let name = self.material.trim();
        if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
            return Err(Error::Request("name the material in 1–120 characters".into()));
        }
        if !self.thickness_mm.is_finite() || self.thickness_mm <= 0. || self.thickness_mm > 1000. {
            return Err(Error::Request("the thickness must be above 0 and at most 1000 mm".into()));
        }
        for side in [self.width_mm, self.height_mm] {
            if !side.is_finite() || side <= 0. || side > MAX_SIDE_MM {
                return Err(Error::Request(format!(
                    "a sheet side must be above 0 and at most {MAX_SIDE_MM} mm"
                )));
            }
        }
        if self.quantity > MAX_QUANTITY {
            return Err(Error::Request(format!("keep at most {MAX_QUANTITY} sheets of a size")));
        }
        Ok(())
    }

    fn same_stock(&self, other: &NewStock) -> bool {
        self.suits(other.laser, &other.material, other.thickness_mm)
            && self.same_size(other.width_mm, other.height_mm)
            && self.folder == other.folder
    }

    /// The same sheet size, either way round: presets turn to fit the bed.
    fn same_size(&self, width: f64, height: f64) -> bool {
        let near = |a: f64, b: f64| (a - b).abs() < SIZE_TOLERANCE_MM;
        (near(self.width_mm, width) && near(self.height_mm, height))
            || (near(self.width_mm, height) && near(self.height_mm, width))
    }
}

/// The recipe rule for stock, shared by sheets and remnants: same laser,
/// material (ignoring case) and thickness.
pub(crate) fn suits(
    laser: LaserMode,
    material: &str,
    thickness_mm: f64,
    recipe_laser: LaserMode,
    recipe_material: &str,
    recipe_thickness_mm: f64,
) -> bool {
    laser == recipe_laser
        && (thickness_mm - recipe_thickness_mm).abs() < 1e-6
        && material.trim().eq_ignore_ascii_case(recipe_material.trim())
}

/// The saved entries; a missing file means none, and an unreadable one is
/// logged and ignored rather than stopping the machine.
#[must_use]
pub fn open(root: &Path) -> Vec<StockItem> {
    let path = root.join(FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "inventory unreadable");
            return Vec::new();
        }
    };
    let loaded =
        serde_json::from_slice::<Saved>(&bytes).map_err(|e| e.to_string()).and_then(|saved| {
            if saved.version != 1 {
                return Err(format!("unsupported version {}", saved.version));
            }
            saved.items.iter().try_for_each(StockItem::validate).map_err(|e| e.to_string())?;
            Ok(saved.items)
        });
    loaded.unwrap_or_else(|error| {
        tracing::warn!(%error, path = %path.display(), "inventory invalid");
        Vec::new()
    })
}

impl Coordinator {
    fn save_inventory(&mut self, items: Vec<StockItem>) -> Result<()> {
        self.write_inventory(items)?;
        self.library_changed();
        Ok(())
    }

    fn write_inventory(&mut self, items: Vec<StockItem>) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&Saved { version: 1, items: items.clone() })
            .map_err(|e| Error::Refused(e.to_string()))?;
        openlaser_library::atomic_write(&self.config.data_dir.join(FILE), &bytes)?;
        self.inventory = items;
        Ok(())
    }

    fn check_folder(&self, folder: Option<&Id>) -> Result<()> {
        match folder {
            Some(id) if !self.library.folders().iter().any(|f| &f.id == id) => {
                Err(Error::Missing("that folder does not exist".into()))
            }
            _ => Ok(()),
        }
    }

    /// Adds sheets to the rack, to a matching entry or a new one.
    pub fn add_stock(&mut self, new: NewStock) -> Result<Id> {
        self.check_folder(new.folder.as_ref())?;
        let mut items = self.inventory.clone();
        let id = if let Some(item) = items.iter_mut().find(|i| i.same_stock(&new)) {
            item.quantity = item.quantity.saturating_add(new.quantity);
            item.validate()?;
            item.id.clone()
        } else {
            let item = StockItem {
                id: Id::generate(),
                material: new.material.trim().to_owned(),
                thickness_mm: new.thickness_mm,
                laser: new.laser,
                width_mm: new.width_mm,
                height_mm: new.height_mm,
                quantity: new.quantity,
                folder: new.folder,
            };
            item.validate()?;
            items.push(item.clone());
            item.id
        };
        self.save_inventory(items)?;
        Ok(id)
    }

    /// Changes one entry's count or folder.
    pub fn change_stock(&mut self, id: &Id, change: StockChange) -> Result<()> {
        if let Some(folder) = &change.folder {
            self.check_folder(folder.as_ref())?;
        }
        let mut items = self.inventory.clone();
        let item = items
            .iter_mut()
            .find(|i| &i.id == id)
            .ok_or_else(|| Error::Missing("those sheets are no longer on the rack".into()))?;
        if let Some(quantity) = change.quantity {
            item.quantity = quantity;
        }
        if let Some(folder) = change.folder {
            item.folder = folder;
        }
        item.validate()?;
        self.save_inventory(items)
    }

    /// Removes one entry.
    pub fn remove_stock(&mut self, id: &Id) -> Result<()> {
        let mut items = self.inventory.clone();
        let before = items.len();
        items.retain(|i| &i.id != id);
        if items.len() == before {
            return Err(Error::Missing("those sheets are no longer on the rack".into()));
        }
        self.save_inventory(items)
    }

    /// Keeps a remnant in a library folder, or the root.
    pub fn set_remnant_folder(&mut self, id: &str, folder: Option<Id>) -> Result<()> {
        self.check_folder(folder.as_ref())?;
        self.sheet_store.set_folder(id, folder)?;
        self.library_changed();
        Ok(())
    }

    /// A cut completed on a full sheet: when its size is on the rack, one
    /// comes off. An entry already at zero is left alone; nothing is refused
    /// over a miscount.
    pub(crate) fn take_sheet(&mut self, plan: &SheetPlan) -> Result<()> {
        let Some([width, height]) = plan.full_sheet() else { return Ok(()) };
        let (laser, material, thickness) = plan.material();
        let mut items = self.inventory.clone();
        let Some(item) = items.iter_mut().find(|i| {
            i.quantity > 0 && i.suits(laser, material, thickness) && i.same_size(width, height)
        }) else {
            return Ok(());
        };
        item.quantity -= 1;
        // Runs while the finished run is published; the caller refreshes the view.
        self.write_inventory(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stock(material: &str) -> StockItem {
        StockItem {
            id: Id::generate(),
            material: material.into(),
            thickness_mm: 2.,
            laser: LaserMode::Fiber,
            width_mm: 1250.,
            height_mm: 2500.,
            quantity: 3,
            folder: None,
        }
    }

    #[test]
    fn stock_suits_only_its_own_laser_material_and_thickness() {
        let sheet = stock("Stainless steel");
        assert!(sheet.suits(LaserMode::Fiber, " stainless STEEL ", 2.));
        assert!(!sheet.suits(LaserMode::Fiber, "Stainless steel", 3.));
        assert!(!sheet.suits(LaserMode::Co2, "Stainless steel", 2.));
        assert!(!sheet.suits(LaserMode::Fiber, "Mild steel", 2.));
    }

    #[test]
    fn impossible_entries_are_refused() {
        assert!(stock("Steel").validate().is_ok());
        assert!(StockItem { width_mm: 0., ..stock("Steel") }.validate().is_err());
        assert!(StockItem { thickness_mm: f64::NAN, ..stock("Steel") }.validate().is_err());
        assert!(StockItem { quantity: MAX_QUANTITY + 1, ..stock("Steel") }.validate().is_err());
        assert!(stock(" ").validate().is_err());
    }
}
