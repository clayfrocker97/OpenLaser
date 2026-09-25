// SPDX-License-Identifier: GPL-3.0-or-later

//! The job's layers, as `LightBurn`'s Cuts / Layers list has them: in the
//! order they run, each with its colour, whether it is output, what it does
//! to the sheet (cut through or mark), the recipe that runs it and, when it
//! has its own, its machining. With more than one layer to output, every
//! layer needs a recipe or to be switched off before the job compiles.
//! Shapes move between layers, and layers can be renamed. A layer's own
//! machining is a features edit, as the job's is.

use crate::coordinator::Coordinator;
use crate::document::RecipeView;
use crate::draft::{Draft, Snapshot, place};
use crate::{Error, Result};
use openlaser_compiler::settings::Settings;
use openlaser_core::features::{Features, Layer, LayerEdit, LayerMode, Machining};
use openlaser_core::geometry::Drawing;
use openlaser_library::{Id, LayerChoice};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// The longest layer name.
const MAX_NAME: usize = 60;

/// One layer of the job, in the order the layers run.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DraftLayer {
    /// Its name.
    pub name: String,
    /// Shapes on it, every copy counted.
    pub contours: usize,
    /// Whether it is cut at all.
    pub output: bool,
    /// It has what it runs under: output off, a recipe chosen, or a cut
    /// through the job's material. A mark among other layers needs its own
    /// recipe, since the material's would cut it through.
    pub chosen: bool,
    /// Its own recipe; a layer without one uses the job's material.
    pub recipe: Option<RecipeView>,
    /// Cut through, or marked on the surface.
    pub mode: LayerMode,
    /// Its colour: the operator's, else the file's; none takes the
    /// screen's cut colour or the next of its palette.
    pub color: Option<[u8; 3]>,
    /// Its own machining, over the job's.
    pub machining: Option<Machining>,
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
    /// Runs a layer with the job's recipe or its own.
    Recipe {
        /// The layer.
        layer: String,
        /// A library recipe; none uses the job's.
        recipe: Option<Id>,
    },
    /// Cuts a layer, or leaves it out.
    Output {
        /// The layer.
        layer: String,
        /// Whether it is cut.
        on: bool,
    },
    /// Cuts a layer through, or marks it on the surface.
    Mode {
        /// The layer.
        layer: String,
        /// What it does.
        mode: LayerMode,
    },
    /// Colours a layer; none goes back to the file's colour.
    Color {
        /// The layer.
        layer: String,
        /// Red, green and blue.
        color: Option<[u8; 3]>,
    },
    /// Sets the order the layers run in.
    Order {
        /// Every layer, first to last.
        layers: Vec<String>,
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

/// The sheet as the layers see it: each placed contour on its layer.
fn sheet(drawing: &Drawing, current: &Snapshot) -> Drawing {
    place(&relayered(drawing, &current.features.layer_edits), &current.placed)
}

/// The job's layers, in the order they run.
pub(crate) fn view(draft: &Draft) -> Vec<DraftLayer> {
    let Ok(drawing) = draft.drawing() else { return Vec::new() };
    let current = &draft.current;
    let sheet = sheet(drawing, current);
    let colors = draft.sources().map_or(&[][..], |s| s.colors());
    let order = openlaser_prep::layer_order(&sheet, &current.features);
    let outputs = order.iter().filter(|name| !current.features.skip_layers.contains(name)).count();
    order
        .into_iter()
        .map(|name| {
            let output = !current.features.skip_layers.contains(&name);
            let choice = current.layers.iter().find(|c| c.layer == name);
            let table = current.features.layer(&name);
            let mode = table.map_or(LayerMode::Cut, |l| l.mode);
            DraftLayer {
                contours: sheet.contours.iter().filter(|c| c.layer == name).count(),
                output,
                chosen: !output || choice.is_some() || mode == LayerMode::Cut || outputs < 2,
                recipe: choice.and_then(|c| c.recipe.as_ref()).map(RecipeView::new),
                mode,
                color: table
                    .and_then(|l| l.color)
                    .or_else(|| colors.iter().find(|c| c.layer == name).map(|c| c.color)),
                machining: table.and_then(|l| l.machining.clone()),
                name,
            }
        })
        .collect()
}

/// Why the job cannot compile yet: every cut runs on the job's material,
/// but a mark among other layers needs its own recipe, or to be switched
/// off. Calibration coupons are OpenLaser's own drawing, one layer per
/// coupon, all on the job's recipe.
pub(crate) fn unchosen(draft: &Draft) -> Option<String> {
    if draft.calibration {
        return None;
    }
    let layers = view(draft);
    let missing: Vec<&str> = layers.iter().filter(|l| !l.chosen).map(|l| l.name.as_str()).collect();
    match missing.as_slice() {
        [] => None,
        [one] => Some(format!("choose a recipe for the mark layer {one}, or turn it off")),
        many => Some(format!(
            "choose a recipe for the mark layers {}, or turn them off",
            many.join(", ")
        )),
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

/// The layer table with every layer in it, in the order they run now, so
/// the order the operator sees is the order that is kept.
fn table<'a>(features: &'a mut Features, sheet: &Drawing, name: &str) -> Result<&'a mut Layer> {
    let order = openlaser_prep::layer_order(sheet, features);
    if !order.iter().any(|l| l == name) {
        return Err(Error::Missing(format!("there is no layer {name}")));
    }
    let mut layers = Vec::with_capacity(order.len());
    for layer in order {
        let entry = features.layers.iter().find(|l| l.name == layer).cloned();
        layers.push(entry.unwrap_or(Layer {
            name: layer,
            mode: LayerMode::Cut,
            color: None,
            machining: None,
        }));
    }
    features.layers = layers;
    features
        .layers
        .iter_mut()
        .find(|l| l.name == name)
        .ok_or_else(|| Error::Missing(format!("there is no layer {name}")))
}

impl Coordinator {
    /// Changes the job's layers as one undo step.
    pub fn change_layers(&mut self, change: LayerChange) -> Result<()> {
        let chosen = match &change {
            LayerChange::Recipe { recipe: Some(id), .. } => Some(self.library.recipe(id)?.clone()),
            _ => None,
        };
        let draft =
            self.draft.as_mut().ok_or_else(|| Error::Refused("open a part first".into()))?;
        let drawing = draft.drawing()?.clone();
        let mut next = draft.current.clone();
        let sheet = sheet(&drawing, &next);
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
            LayerChange::Rename { from, to } => rename(&mut next, &drawing, &from, &to)?,
            LayerChange::Recipe { layer, .. } => {
                if let (Some(recipe), Some(job)) = (&chosen, &next.recipe)
                    && recipe.laser != job.laser
                {
                    return Err(Error::Request("choose a recipe for this job's laser".into()));
                }
                next.layers.retain(|c| c.layer != layer);
                next.layers.push(LayerChoice { layer, recipe: chosen });
            }
            LayerChange::Output { layer, on } => {
                next.features.skip_layers.retain(|l| *l != layer);
                if !on {
                    next.features.skip_layers.push(layer);
                }
            }
            LayerChange::Mode { layer, mode } => {
                table(&mut next.features, &sheet, &layer)?.mode = mode;
            }
            LayerChange::Color { layer, color } => {
                table(&mut next.features, &sheet, &layer)?.color = color;
            }
            LayerChange::Order { layers } => {
                let mut listed = layers.clone();
                listed.sort();
                let mut present = openlaser_prep::layer_order(&sheet, &next.features);
                present.sort();
                if listed != present {
                    return Err(Error::Request("list every layer once to reorder them".into()));
                }
                table(&mut next.features, &sheet, &layers[0])?;
                let mut ordered = Vec::with_capacity(layers.len());
                for name in layers {
                    if let Some(at) = next.features.layers.iter().position(|l| l.name == name) {
                        ordered.push(next.features.layers.remove(at));
                    }
                }
                next.features.layers = ordered;
            }
        }
        draft.remember();
        draft.current = next;
        self.reprepare();
        Ok(())
    }
}

/// Renames a layer everywhere the job names it: its shapes, whether it is
/// output, its recipe and its table entry. Onto an existing name the two
/// layers join, and the one kept keeps its settings.
fn rename(next: &mut Snapshot, drawing: &Drawing, from: &str, to: &str) -> Result<()> {
    let to = check_name(to)?;
    let moved = relayered(drawing, &next.features.layer_edits);
    let sources: Vec<usize> =
        (0..moved.contours.len()).filter(|&i| moved.contours[i].layer == from).collect();
    if sources.is_empty() {
        return Err(Error::Missing(format!("there is no layer {from}")));
    }
    for source in sources {
        move_shape(&mut next.features, drawing, source, &to);
    }
    let joins = next.features.layer(&to).is_some() || next.layers.iter().any(|c| c.layer == to);
    let skipped = next.features.skip_layers.iter().any(|l| l == from);
    next.features.skip_layers.retain(|l| l != from && *l != to);
    if skipped {
        next.features.skip_layers.push(to.clone());
    }
    if joins {
        next.layers.retain(|c| c.layer != from);
        next.features.layers.retain(|l| l.name != from);
    } else {
        if let Some(choice) = next.layers.iter_mut().find(|c| c.layer == from) {
            choice.layer.clone_from(&to);
        }
        if let Some(layer) = next.features.layers.iter_mut().find(|l| l.name == from) {
            layer.name = to;
        }
    }
    Ok(())
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

    fn line(layer: &str, y: f64, length: f64) -> Contour {
        Contour {
            layer: layer.into(),
            curves: vec![Curve::Line { start: Point::new(0., y), end: Point::new(length, y + 1.) }],
        }
    }

    #[test]
    fn moved_shapes_take_their_new_layer_and_the_rest_keep_theirs() {
        let drawing = Drawing { contours: vec![line("0", 0., 10.), line("0", 5., 10.)] };
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
    fn a_change_to_one_layer_keeps_every_layer_in_the_order_they_run() {
        let sheet = Drawing { contours: vec![line("Big", 0., 50.), line("Small", 5., 5.)] };
        let mut features = Features::default();
        table(&mut features, &sheet, "Big").unwrap().mode = LayerMode::Mark;
        let names: Vec<_> = features.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Small", "Big"], "the order seen is the order kept");
        assert_eq!(features.mode("Big"), LayerMode::Mark);
        assert!(table(&mut features, &sheet, "Nope").is_err());
    }
}
