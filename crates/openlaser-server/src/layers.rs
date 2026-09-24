// SPDX-License-Identifier: GPL-3.0-or-later

//! Drawing layers as a job cuts them. The operator can move shapes to
//! another layer, name layers, and choose for each layer the recipe that
//! cuts it, whether it is engraved on the surface, or that it is left
//! uncut. With more than one layer to cut, every layer needs that choice
//! before the job compiles.

use crate::coordinator::Coordinator;
use crate::document::RecipeView;
use crate::draft::Draft;
use crate::{Error, Result};
use openlaser_compiler::settings::Settings;
use openlaser_core::features::{Features, LayerEdit, OrderStrategy};
use openlaser_core::geometry::Drawing;
use openlaser_core::toolpath::Toolpath;
use openlaser_library::{Id, LayerChoice};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// The longest layer name.
const MAX_NAME: usize = 60;

/// One layer of the job.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DraftLayer {
    /// Its name.
    pub name: String,
    /// Shapes on it, every copy counted.
    pub contours: usize,
    /// Left uncut.
    pub ignored: bool,
    /// The operator chose how it is cut: a recipe, or ignored.
    pub chosen: bool,
    /// Its own recipe; a chosen layer without one uses the job's.
    pub recipe: Option<RecipeView>,
    /// Engraved on the surface, ahead of the cuts.
    pub engrave: bool,
}

/// A change to the job's layers; each is one undo step.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayerChange {
    /// Moves shapes, as placed contours, to a layer, new or existing.
    Assign {
        /// Placed contours; every copy of each shape moves with it.
        contours: Vec<usize>,
        /// The layer they go to.
        layer: String,
    },
    /// Renames a layer; onto an existing name, the two join.
    Rename {
        /// Its name now.
        from: String,
        /// Its new name.
        to: String,
    },
    /// Cuts a layer with the job's recipe or its own.
    Cut {
        /// The layer.
        layer: String,
        /// A library recipe; none uses the job's.
        recipe: Option<Id>,
        /// Engraved on the surface instead of cut through.
        engrave: bool,
    },
    /// Leaves a layer uncut.
    Ignore {
        /// The layer.
        layer: String,
    },
}

/// The drawing with the shapes the operator moved on their new layers.
pub(crate) fn relayered<'a>(drawing: &'a Drawing, edits: &[LayerEdit]) -> Cow<'a, Drawing> {
    if edits.is_empty() {
        return Cow::Borrowed(drawing);
    }
    let mut moved = drawing.clone();
    for edit in edits {
        if let Some(contour) = moved.contours.get_mut(edit.contour) {
            contour.layer.clone_from(&edit.layer);
        }
    }
    Cow::Owned(moved)
}

/// The layers engraved rather than cut through.
pub(crate) fn engraved(choices: &[LayerChoice]) -> Vec<String> {
    choices.iter().filter(|c| c.engrave).map(|c| c.layer.clone()).collect()
}

/// Prepares a sheet. Engraved layers come first as bare lines: no kerf,
/// leads, joints or cooling, and they never make a hole or an outline of
/// the parts cut through after them.
pub(crate) fn prepare(
    sheet: &Drawing,
    features: &Features,
    engraved: &[String],
) -> Result<Toolpath> {
    let cut = |layer: &String| !features.skip_layers.contains(layer);
    let marks: Vec<String> = engraved
        .iter()
        .filter(|layer| cut(layer) && sheet.contours.iter().any(|c| &c.layer == *layer))
        .cloned()
        .collect();
    if marks.is_empty() {
        return Ok(openlaser_prep::prepare(sheet, features)?);
    }
    let mut others: Vec<String> = Vec::new();
    for contour in &sheet.contours {
        if !marks.contains(&contour.layer) && !others.contains(&contour.layer) {
            others.push(contour.layer.clone());
        }
    }
    let mut order = features.order.clone();
    if matches!(order.strategy, OrderStrategy::Manual(_)) {
        order.strategy = OrderStrategy::default();
    }
    let surface = Features {
        skip_layers: others.iter().chain(&features.skip_layers).cloned().collect(),
        order,
        ..Features::default()
    };
    let mut toolpath = openlaser_prep::prepare(sheet, &surface)?;
    if others.iter().any(cut) {
        let through = Features {
            skip_layers: features.skip_layers.iter().chain(&marks).cloned().collect(),
            ..features.clone()
        };
        let cuts = openlaser_prep::prepare(sheet, &through)?;
        toolpath.contours.extend(cuts.contours);
        toolpath.warnings.extend(cuts.warnings);
    }
    Ok(toolpath)
}

/// Each shape's layer, placed contour by placed contour.
fn placed_layers(draft: &Draft) -> Vec<String> {
    let Ok(drawing) = draft.drawing() else { return Vec::new() };
    let drawing = relayered(drawing, &draft.current.features.layer_edits);
    draft
        .current
        .placed
        .iter()
        .filter_map(|p| drawing.contours.get(p.source).map(|c| c.layer.clone()))
        .collect()
}

/// The job's layers, in the order the drawing first uses them.
pub(crate) fn view(draft: &Draft) -> Vec<DraftLayer> {
    let mut layers: Vec<DraftLayer> = Vec::new();
    for name in placed_layers(draft) {
        if let Some(layer) = layers.iter_mut().find(|l| l.name == name) {
            layer.contours += 1;
            continue;
        }
        let ignored = draft.current.features.skip_layers.contains(&name);
        let choice = draft.current.layers.iter().find(|c| c.layer == name);
        layers.push(DraftLayer {
            contours: 1,
            ignored,
            chosen: ignored || choice.is_some(),
            recipe: choice.and_then(|c| c.recipe.as_ref()).map(RecipeView::new),
            engrave: choice.is_some_and(|c| c.engrave),
            name,
        });
    }
    layers
}

/// Why the job cannot compile yet: with more than one layer to cut, each
/// needs a recipe or to be ignored. Calibration coupons are OpenLaser's
/// own drawing, one layer per coupon, all cut with the job's recipe.
pub(crate) fn unchosen(draft: &Draft) -> Option<String> {
    if draft.calibration {
        return None;
    }
    let layers = view(draft);
    let cut: Vec<_> = layers.iter().filter(|l| !l.ignored).collect();
    if cut.len() < 2 {
        return None;
    }
    let missing: Vec<&str> = cut.iter().filter(|l| !l.chosen).map(|l| l.name.as_str()).collect();
    match missing.as_slice() {
        [] => None,
        [one] => Some(format!("choose a recipe or Ignore for layer {one}")),
        many => Some(format!("choose a recipe or Ignore for layers {}", many.join(", "))),
    }
}

/// Which recipe cuts each prepared contour: none for the job's own, or an
/// index into `settings`, the layers' own recipes bound.
#[derive(Clone, Debug, Default)]
pub struct Layered {
    /// The layers' own recipes, bound.
    pub settings: Vec<Settings>,
    /// Per prepared contour, its recipe among `settings`, or the job's.
    pub contours: Vec<Option<usize>>,
}

impl Layered {
    /// Per contour, the settings it runs under; none when every contour
    /// uses the job's recipe.
    #[must_use]
    pub fn processes<'a>(&'a self, job: &'a Settings) -> Option<Vec<&'a Settings>> {
        self.contours
            .iter()
            .any(Option::is_some)
            .then(|| self.contours.iter().map(|at| at.map_or(job, |i| &self.settings[i])).collect())
    }
}

/// The layers' own recipes bound with `bind`, and which one cuts each
/// prepared contour, by the layer its preview names.
pub(crate) fn layered(
    draft: &Draft,
    prepared: &crate::draft::Prepared,
    bind: impl Fn(&openlaser_library::Recipe) -> Result<Settings>,
) -> Result<Layered> {
    if let Some(reason) = unchosen(draft) {
        return Err(Error::Refused(reason));
    }
    let job = draft.current.recipe.as_ref().map(|r| r.laser);
    let mut layered = Layered::default();
    let mut bound: Vec<&str> = Vec::new();
    for choice in &draft.current.layers {
        let Some(recipe) = &choice.recipe else { continue };
        if job.is_some_and(|laser| laser != recipe.laser) {
            return Err(Error::Refused(format!(
                "layer {}'s recipe is for the other laser",
                choice.layer
            )));
        }
        layered.settings.push(bind(recipe)?);
        bound.push(&choice.layer);
    }
    layered.contours = prepared
        .preview
        .contours
        .iter()
        .map(|c| bound.iter().position(|layer| *layer == c.layer))
        .collect();
    Ok(layered)
}

fn check_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(Error::Request(format!("name the layer in 1–{MAX_NAME} characters")));
    }
    Ok(name.to_owned())
}

impl Coordinator {
    /// Changes the job's layers as one undo step.
    pub fn change_layers(&mut self, change: LayerChange) -> Result<()> {
        let chosen = match &change {
            LayerChange::Cut { recipe: Some(id), .. } => Some(self.library.recipe(id)?.clone()),
            _ => None,
        };
        let draft =
            self.draft.as_mut().ok_or_else(|| Error::Refused("open a part first".into()))?;
        let drawing = draft.drawing()?.clone();
        let mut next = draft.current.clone();
        match change {
            LayerChange::Assign { contours, layer } => {
                let layer = check_name(&layer)?;
                for placed in contours {
                    let source = next
                        .placed
                        .get(placed)
                        .ok_or_else(|| Error::Request("select shapes on the drawing".into()))?
                        .source;
                    move_shape(&mut next.features, &drawing, source, &layer);
                }
            }
            LayerChange::Rename { from, to } => {
                let to = check_name(&to)?;
                let moved = relayered(&drawing, &next.features.layer_edits);
                let sources: Vec<usize> = (0..moved.contours.len())
                    .filter(|&i| moved.contours[i].layer == from)
                    .collect();
                if sources.is_empty() {
                    return Err(Error::Missing(format!("there is no layer {from}")));
                }
                for source in sources {
                    move_shape(&mut next.features, &drawing, source, &to);
                }
                let skipped = next.features.skip_layers.contains(&from);
                next.features.skip_layers.retain(|l| *l != from && *l != to);
                if skipped {
                    next.features.skip_layers.push(to.clone());
                }
                if next.layers.iter().any(|c| c.layer == to) {
                    next.layers.retain(|c| c.layer != from);
                } else if let Some(choice) = next.layers.iter_mut().find(|c| c.layer == from) {
                    choice.layer = to;
                }
            }
            LayerChange::Cut { layer, engrave, .. } => {
                if let (Some(recipe), Some(job)) = (&chosen, &next.recipe)
                    && recipe.laser != job.laser
                {
                    return Err(Error::Request("choose a recipe for this job's laser".into()));
                }
                next.features.skip_layers.retain(|l| *l != layer);
                next.layers.retain(|c| c.layer != layer);
                next.layers.push(LayerChoice { layer, recipe: chosen, engrave });
            }
            LayerChange::Ignore { layer } => {
                next.layers.retain(|c| c.layer != layer);
                if !next.features.skip_layers.contains(&layer) {
                    next.features.skip_layers.push(layer);
                }
            }
        }
        draft.remember();
        draft.current = next;
        self.reprepare();
        Ok(())
    }
}

/// Puts one drawing contour on `layer`, and forgets the change when that
/// is the layer the drawing gives it.
fn move_shape(features: &mut Features, drawing: &Drawing, source: usize, layer: &str) {
    features.layer_edits.retain(|e| e.contour != source);
    if drawing.contours.get(source).is_some_and(|c| c.layer != layer) {
        features.layer_edits.push(LayerEdit { contour: source, layer: layer.to_owned() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_core::geometry::{Contour, Curve, Point};

    fn line(layer: &str, y: f64) -> Contour {
        Contour {
            layer: layer.into(),
            curves: vec![Curve::Line { start: Point::new(0., y), end: Point::new(10., y) }],
        }
    }

    #[test]
    fn moved_shapes_take_their_new_layer_and_the_rest_keep_theirs() {
        let drawing = Drawing { contours: vec![line("0", 0.), line("0", 5.)] };
        assert!(matches!(relayered(&drawing, &[]), Cow::Borrowed(_)));
        let edits = [LayerEdit { contour: 1, layer: "Bend".into() }];
        let moved = relayered(&drawing, &edits);
        assert_eq!(moved.contours[0].layer, "0");
        assert_eq!(moved.contours[1].layer, "Bend");
        let mut features = Features { layer_edits: edits.to_vec(), ..Features::default() };
        move_shape(&mut features, &drawing, 1, "0");
        assert!(features.layer_edits.is_empty(), "back on its own layer, nothing to remember");
    }

    #[test]
    fn engraved_layers_come_first_as_bare_lines() {
        let sheet = Drawing { contours: vec![line("Cut", 0.), line("Mark", 5.), line("Skip", 9.)] };
        let features = Features { skip_layers: vec!["Skip".into()], ..Features::default() };
        let toolpath = prepare(&sheet, &features, &["Mark".into()]).unwrap();
        let layers: Vec<_> = toolpath.contours.iter().map(|c| c.layer.as_str()).collect();
        assert_eq!(layers, ["Mark", "Cut"]);
        let only = prepare(&sheet, &features, &["Mark".into(), "Cut".into()]).unwrap();
        assert_eq!(only.contours.len(), 2, "nothing is left to cut through, and that is fine");
    }
}
