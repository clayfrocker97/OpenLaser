// SPDX-License-Identifier: GPL-3.0-or-later

//! The machining features an operator chooses for a job: how contours are
//! entered and left, held, cooled, compensated, bridged and ordered.
//!
//! These are persisted in jobs, so every type serialises; absent features
//! read as off, and new fields must carry defaults.

use crate::geometry::Point;
use crate::units::{Degrees, Millimeters, Milliseconds, MmPerSecond, Percent};
use serde::{Deserialize, Serialize};

/// Everything preparation needs beyond the drawing.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Features {
    /// Entry and exit leads.
    pub leads: Option<Leads>,
    /// Micro-joints that hold parts in the sheet.
    pub joints: Option<Joints>,
    /// Cooling stops.
    pub cooling: Option<Cooling>,
    /// Kerf compensation.
    pub kerf: Option<Kerf>,
    /// Bridges between contours.
    pub bridges: Option<Bridges>,
    /// Share compatible coincident spans between these placed contours.
    pub common: Option<CommonEdges>,
    /// Where each contour starts and which way it runs.
    pub start: Start,
    /// How a closed contour's seam is treated.
    pub seam: Seam,
    /// The cutting order.
    pub order: CutOrder,
    /// Drawing layers left uncut.
    pub skip_layers: Vec<String>,
    /// Shapes moved to another layer than the drawing gives them; left
    /// out of the file when there are none, as earlier builds wrote it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<Vec<LayerEdit>>", optional))]
    pub layer_edits: Vec<LayerEdit>,
    /// The layers in the order they run, with what each does and its own
    /// colour and machining. Empty until the operator changes a layer.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<Vec<Layer>>", optional))]
    pub layers: Vec<Layer>,
}

/// What a layer does to the sheet.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerMode {
    /// Cut through: its closed shapes are the parts and their holes.
    #[default]
    Cut,
    /// Traced on the surface: never a part or a hole, and without the
    /// job's machining unless the layer has its own.
    Mark,
}

/// One of the job's layers.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Its name.
    pub name: String,
    /// What it does to the sheet.
    #[serde(default)]
    pub mode: LayerMode,
    /// Its colour on screen, over the drawing's own.
    #[serde(default)]
    pub color: Option<[u8; 3]>,
    /// Its own machining, over the job's.
    #[serde(default)]
    pub machining: Option<Machining>,
}

/// What happens along a layer's contours, when the layer has its own.
/// Picked places, such as manual starts, joints and lead edits, stay the
/// job's, so a layer's manual joints are the job's manual joints.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Machining {
    /// Entry and exit leads.
    pub leads: Option<Leads>,
    /// Micro-joints.
    pub joints: Option<Joints>,
    /// Cooling stops.
    pub cooling: Option<Cooling>,
    /// Kerf compensation.
    pub kerf: Option<Kerf>,
    /// Where each contour starts and which way it runs.
    pub start: Start,
    /// How a closed contour's seam is treated.
    pub seam: Seam,
}

/// One shape on a layer of the operator's choosing. The shape is a contour
/// of the job's drawing, so every copy of it follows.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerEdit {
    /// The drawing contour.
    pub contour: usize,
    /// The layer it is on.
    pub layer: String,
}

impl Features {
    /// The layer's entry in the table, when it has one.
    #[must_use]
    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name)
    }

    /// What the layer does to the sheet.
    #[must_use]
    pub fn mode(&self, name: &str) -> LayerMode {
        self.layer(name).map_or(LayerMode::Cut, |l| l.mode)
    }

    /// The features a layer's contours are prepared with: its own
    /// machining over the job's, keeping the job's picked places. A marked
    /// layer without machining of its own has none.
    #[must_use]
    pub fn for_layer(&self, name: &str) -> std::borrow::Cow<'_, Self> {
        let Some(layer) = self.layer(name) else { return std::borrow::Cow::Borrowed(self) };
        let own = match (&layer.machining, layer.mode) {
            (Some(machining), _) => machining.clone(),
            (None, LayerMode::Mark) => Machining::default(),
            (None, LayerMode::Cut) => return std::borrow::Cow::Borrowed(self),
        };
        let mut features = self.clone();
        features.leads = own.leads.map(|mut leads| {
            leads.overrides = self.leads.as_ref().map(|l| l.overrides.clone()).unwrap_or_default();
            leads
        });
        features.joints = own.joints.map(|mut joints| {
            if matches!(joints.placement, JointPlacement::Manual(_)) {
                joints.placement = match self.joints.as_ref().map(|j| &j.placement) {
                    Some(manual @ JointPlacement::Manual(_)) => manual.clone(),
                    _ => JointPlacement::Manual(Vec::new()),
                };
            }
            joints
        });
        features.cooling = own.cooling.map(|mut cooling| {
            if matches!(cooling.placement, CoolingPlacement::Manual(_)) {
                cooling.placement = match self.cooling.as_ref().map(|c| &c.placement) {
                    Some(manual @ CoolingPlacement::Manual(_)) => manual.clone(),
                    _ => CoolingPlacement::Manual(Vec::new()),
                };
            }
            cooling
        });
        features.kerf = own.kerf;
        features.start = Start { spots: self.start.spots.clone(), ..own.start };
        features.seam = own.seam;
        std::borrow::Cow::Owned(features)
    }

    /// Machining defaults for another drawing. Widths, speeds and automatic
    /// strategies carry over; picked locations and layer choices do not.
    #[must_use]
    pub fn reusable(&self) -> Self {
        let mut features = self.renumbered_locations(|_| None);
        features.common = None;
        if matches!(features.start.position, StartPosition::Manual(_)) {
            features.start.position = StartPosition::default();
        }
        if matches!(features.order.strategy, OrderStrategy::Manual(_)) {
            features.order.strategy = OrderStrategy::default();
        }
        if let Some(bridges) = &mut features.bridges {
            bridges.connections.clear();
        }
        features.skip_layers.clear();
        features.layer_edits.clear();
        features.layers.clear();
        features
    }

    /// Renumber manual spots and cut order by placed contour identity,
    /// dropping locations on removed contours. Bridge references name an
    /// evolving topology; their remapping belongs to the preparation crate.
    #[must_use]
    pub fn renumbered_locations(&self, renumber: impl Fn(usize) -> Option<usize>) -> Self {
        let mut features = self.clone();
        let keep = |spots: &mut Vec<Spot>| {
            spots.retain_mut(|spot| match renumber(spot.contour) {
                Some(contour) => {
                    spot.contour = contour;
                    true
                }
                None => false,
            });
        };
        if let Some(joints) = &mut features.joints
            && let JointPlacement::Manual(spots) = &mut joints.placement
        {
            keep(spots);
        }
        if let Some(cooling) = &mut features.cooling
            && let CoolingPlacement::Manual(spots) = &mut cooling.placement
        {
            keep(spots);
        }
        keep(&mut features.start.spots);
        if let Some(leads) = &mut features.leads {
            leads.overrides.retain_mut(|lead| match renumber(lead.location.contour) {
                Some(contour) => {
                    lead.location.contour = contour;
                    true
                }
                None => false,
            });
        }
        if let Some(common) = &mut features.common {
            common.contours = common.contours.iter().filter_map(|&i| renumber(i)).collect();
            if common.contours.len() < 2 {
                features.common = None;
            }
        }
        if let OrderStrategy::Manual(order) = &mut features.order.strategy {
            *order = order.iter().filter_map(|&contour| renumber(contour)).collect();
        }
        features
    }
}

/// Selected contours whose shared spans can be cut once.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommonEdges {
    /// Placed contour identities, before bridges.
    pub contours: Vec<usize>,
    /// Maximum separation for equivalent spans, in millimetres.
    pub tolerance: f64,
    /// Permit retracing a shared span to connect remaining cuts on one contour.
    pub allow_overcut: bool,
}

/// A place along a drawing contour: the contour's index in the drawing and
/// the fraction of its length, as drawn.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    /// The contour.
    pub contour: usize,
    /// How far along it, 0 to 1.
    pub fraction: f64,
}

/// Entry and exit leads.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Leads {
    /// The lead-in, if any.
    pub entry: Option<Lead>,
    /// The lead-out, if any.
    pub exit: Option<Lead>,
    /// Which side of the contour the leads lie on.
    pub side: Side,
    /// Leave open contours without leads.
    pub closed_only: bool,
    /// Leads edited on the drawing. Locations name placed contours and
    /// distinguish pieces created by a bridge split.
    pub overrides: Vec<LeadOverride>,
}

impl Leads {
    /// Resolve one contour's edit against the job defaults. Entry/exit
    /// switches still disable that role across the job, including overrides.
    #[must_use]
    pub fn resolved(&self, edited: Option<&LeadOverride>) -> Self {
        Self {
            entry: self.entry.map(|base| edited.and_then(|v| v.entry).unwrap_or(base)),
            exit: self.exit.map(|base| edited.and_then(|v| v.exit).unwrap_or(base)),
            side: self.side,
            closed_only: self.closed_only,
            overrides: Vec::new(),
        }
    }
}

/// A lead edit anchored to one placed contour. The retained location follows
/// that contour through moves, copies and bridge operations; it does not
/// change the cutting start. A missing role inherits its job default.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeadOverride {
    /// A retained point on the owning source contour, before compensation.
    pub location: Spot,
    /// The local entry definition, if edited.
    #[serde(default)]
    pub entry: Option<Lead>,
    /// The local exit definition, if edited.
    #[serde(default)]
    pub exit: Option<Lead>,
}

/// One lead.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lead {
    /// Its shape.
    pub shape: LeadShape,
    /// The length of the straight part.
    pub length: Millimeters,
    /// The radius of the arc part.
    pub radius: Millimeters,
    /// The angle between the lead and the contour, or the arc's sweep.
    pub angle: Degrees,
}

/// The shape of a lead.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeadShape {
    /// A straight line at an angle to the contour.
    Line,
    /// An arc tangent to the contour.
    Arc,
    /// A straight line into an arc tangent to the contour.
    LineArc,
}

/// Which side of a contour something lies on.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// Inside for holes, outside for outlines, by nesting depth.
    #[default]
    Auto,
    /// Inside the contour.
    Inside,
    /// Outside the contour.
    Outside,
}

/// Micro-joints: short gaps in the cut that hold a part in the sheet.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Joints {
    /// Where the joints go.
    pub placement: JointPlacement,
    /// How wide each joint is along the contour.
    pub width: Millimeters,
    /// Contours smaller than this in both directions get no joints.
    pub minimum_size: Millimeters,
    /// Joint only outlines, never holes.
    pub outer_only: bool,
    /// On an open contour, put a joint at its start as well.
    #[serde(default)]
    pub open_start: bool,
    /// How the laser behaves in a joint.
    pub behaviour: JointBehaviour,
    /// A slower speed through the joint.
    pub slow_speed: Option<MmPerSecond>,
    /// Pierce again after each joint.
    pub repierce: bool,
}

/// Where micro-joints go.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JointPlacement {
    /// So many, evenly spread along the contour.
    Count(u32),
    /// Every so far along the contour.
    Spacing(Millimeters),
    /// At grid lines across the X axis, so many between the contour's ends.
    AcrossX(u32),
    /// At grid lines across the Y axis, so many between the contour's ends.
    AcrossY(u32),
    /// At these places, each on its own contour.
    Manual(Vec<Spot>),
}

/// What the laser does in a joint.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JointBehaviour {
    /// Off.
    #[default]
    LaserOff,
    /// On at a fraction of the recipe's power, leaving a trace.
    Power(Percent),
}

/// Cooling stops: the head waits with the laser off.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cooling {
    /// How long each stop lasts.
    pub dwell: Milliseconds,
    /// Where the stops go.
    pub placement: CoolingPlacement,
}

/// Where cooling stops go.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoolingPlacement {
    /// Chosen from the geometry.
    Automatic {
        /// A stop where the contour starts.
        at_start: bool,
        /// A stop at every corner sharper than this interior angle.
        corners_below: Option<Degrees>,
    },
    /// At these places, each on its own contour.
    Manual(Vec<Spot>),
}

/// Kerf compensation: the path moves half the kerf into the waste.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Kerf {
    /// The width the beam removes.
    pub width: Millimeters,
    /// Which side the waste is on.
    pub side: Side,
}

/// Bridges: two contours joined through a channel so they cut as one.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bridges {
    /// The channel width.
    pub width: Millimeters,
    /// The connections, applied in order.
    pub connections: Vec<Bridge>,
}

/// One bridge, between two picked points.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bridge {
    /// One end.
    pub first: Pick,
    /// The other end.
    pub second: Pick,
}

/// A point picked on a contour.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pick {
    /// The contour, as an index into the contours at the time of the pick.
    pub contour: usize,
    /// Point in the local coordinates of the result's lowest original
    /// placed owner; preparation resolves that owner through prior bridges.
    pub point: Point,
}

/// Where each contour starts and which way it runs.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Start {
    /// Where cutting starts.
    pub position: StartPosition,
    /// Which way the cut runs.
    pub direction: Direction,
    /// Starts chosen on the drawing, one per contour, over the position.
    #[serde(default)]
    pub spots: Vec<Spot>,
}

/// Where a contour's cut starts.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartPosition {
    /// Where the drawing starts it.
    #[default]
    Keep,
    /// Halfway along the longest segment of a closed contour.
    Automatic,
    /// At this fraction of the contour's length, as drawn.
    Manual(f64),
}

/// Which way a cut runs.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// As drawn.
    #[default]
    Keep,
    /// Clockwise around closed contours.
    Clockwise,
    /// Counterclockwise around closed contours.
    Counterclockwise,
    /// The opposite of how it was drawn.
    Reverse,
}

/// How a closed contour's seam, where the cut ends where it began, is cut.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Seam {
    /// Cut exactly around.
    #[default]
    Seal,
    /// Stop this far short, leaving the part attached.
    Gap(Millimeters),
    /// Cut this far past the start again.
    Overcut(Millimeters),
}

/// The cutting order.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CutOrder {
    /// The base order.
    pub strategy: OrderStrategy,
    /// Cut holes before the outline around them.
    pub inner_first: bool,
    /// Cut circles before other shapes.
    pub circles_first: bool,
    /// Keep consecutive cuts apart to spread heat.
    pub spread_heat: bool,
}

/// The base cutting order.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStrategy {
    /// As drawn.
    #[default]
    AsDrawn,
    /// By position, left to right.
    LeftToRight,
    /// By position, right to left.
    RightToLeft,
    /// By position, bottom to top.
    BottomToTop,
    /// By position, top to bottom.
    TopToBottom,
    /// Each next contour the nearest to the last.
    Nearest,
    /// Drawing contours in this order.
    Manual(Vec<usize>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layer_takes_its_own_machining_and_keeps_the_jobs_picked_places() {
        let spot = Spot { contour: 3, fraction: 0.25 };
        let kerf = Kerf { width: Millimeters(0.2), side: Side::Auto };
        let job = Features {
            kerf: Some(kerf),
            start: Start { spots: vec![spot], ..Start::default() },
            layers: vec![
                Layer { name: "Mark".into(), mode: LayerMode::Mark, color: None, machining: None },
                Layer {
                    name: "Slots".into(),
                    mode: LayerMode::Cut,
                    color: None,
                    machining: Some(Machining {
                        seam: Seam::Overcut(Millimeters(1.)),
                        ..Machining::default()
                    }),
                },
            ],
            ..Features::default()
        };
        assert_eq!(*job.for_layer("0"), job, "a layer without an entry cuts as the job");
        let mark = job.for_layer("Mark");
        assert!(mark.kerf.is_none(), "a mark has no machining of its own, so none");
        assert_eq!(mark.start.spots, [spot]);
        let slots = job.for_layer("Slots");
        assert!(slots.kerf.is_none() && slots.seam == Seam::Overcut(Millimeters(1.)));
        assert_eq!(job.mode("Mark"), LayerMode::Mark);
        assert_eq!(job.mode("0"), LayerMode::Cut);
    }

    #[test]
    fn reusable_defaults_keep_settings_without_drawing_anchors() {
        let spot = Spot { contour: 3, fraction: 0.4 };
        let features = Features {
            cooling: Some(Cooling {
                dwell: Milliseconds(300),
                placement: CoolingPlacement::Manual(vec![spot]),
            }),
            bridges: Some(Bridges {
                width: Millimeters(0.5),
                connections: vec![Bridge {
                    first: Pick { contour: 0, point: Point::ORIGIN },
                    second: Pick { contour: 1, point: Point::new(1., 0.) },
                }],
            }),
            start: Start {
                position: StartPosition::Manual(0.5),
                direction: Direction::Clockwise,
                spots: vec![spot],
            },
            order: CutOrder {
                strategy: OrderStrategy::Manual(vec![1, 0]),
                inner_first: true,
                ..CutOrder::default()
            },
            skip_layers: vec!["construction".into()],
            ..Features::default()
        };
        let defaults = features.reusable();
        assert!(defaults.bridges.as_ref().unwrap().connections.is_empty());
        assert_eq!(defaults.bridges.unwrap().width, Millimeters(0.5));
        assert_eq!(
            defaults.cooling.unwrap(),
            Cooling { dwell: Milliseconds(300), placement: CoolingPlacement::Manual(vec![]) }
        );
        assert!(defaults.start.spots.is_empty() && defaults.skip_layers.is_empty());
        assert_eq!(defaults.start.direction, Direction::Clockwise);
        assert_eq!(defaults.start.position, StartPosition::Keep);
        assert_eq!(defaults.order.strategy, OrderStrategy::AsDrawn);
        assert!(defaults.order.inner_first);
        assert_eq!(features.start.spots, [spot], "the saved job keeps its anchors");
    }

    /// A job file round-trips, absent features read as off, and an older
    /// file without newer fields still loads.
    #[test]
    fn features_round_trip_and_default_to_off() {
        let features = Features {
            leads: Some(Leads {
                entry: Some(Lead {
                    shape: LeadShape::LineArc,
                    length: Millimeters(2.),
                    radius: Millimeters(1.),
                    angle: Degrees(90.),
                }),
                exit: None,
                side: Side::Auto,
                closed_only: true,
                overrides: Vec::new(),
            }),
            joints: Some(Joints {
                placement: JointPlacement::Count(2),
                width: Millimeters(0.4),
                minimum_size: Millimeters(40.),
                outer_only: true,
                open_start: false,
                behaviour: JointBehaviour::Power(Percent(20.)),
                slow_speed: None,
                repierce: false,
            }),
            cooling: Some(Cooling {
                dwell: Milliseconds(300),
                placement: CoolingPlacement::Automatic {
                    at_start: true,
                    corners_below: Some(Degrees(60.)),
                },
            }),
            kerf: Some(Kerf { width: Millimeters(0.2), side: Side::Auto }),
            bridges: None,
            common: None,
            start: Start {
                position: StartPosition::Manual(0.25),
                direction: Direction::Clockwise,
                spots: vec![Spot { contour: 1, fraction: 0.5 }],
            },
            seam: Seam::Overcut(Millimeters(1.)),
            order: CutOrder {
                strategy: OrderStrategy::Nearest,
                inner_first: true,
                ..CutOrder::default()
            },
            skip_layers: vec!["NOTES".into()],
            layer_edits: vec![LayerEdit { contour: 2, layer: "Etch".into() }],
            layers: vec![Layer {
                name: "Etch".into(),
                mode: LayerMode::Mark,
                color: Some([255, 0, 0]),
                machining: Some(Machining::default()),
            }],
        };
        let json = serde_json::to_string(&features).unwrap();
        assert_eq!(serde_json::from_str::<Features>(&json).unwrap(), features);
        assert_eq!(serde_json::from_str::<Features>("{}").unwrap(), Features::default());
        let partial: Features =
            serde_json::from_str(r#"{"kerf":{"width":0.3,"side":"inside"}}"#).unwrap();
        assert_eq!(partial.kerf, Some(Kerf { width: Millimeters(0.3), side: Side::Inside }));
        assert_eq!(partial.order.strategy, OrderStrategy::AsDrawn);
    }

    /// Renumbering drops what lay on a contour that is gone and shifts the
    /// rest: without contour 1, its joint, its bridge and its place in the
    /// order go with it, and contour 2 becomes 1.
    #[test]
    fn renumbering_drops_the_gone_and_shifts_the_rest() {
        let spot = |contour| Spot { contour, fraction: 0.5 };
        let pick = |contour| Pick { contour, point: Point::ORIGIN };
        let features = Features {
            joints: Some(Joints {
                placement: JointPlacement::Manual(vec![spot(0), spot(1), spot(2)]),
                width: Millimeters(0.4),
                minimum_size: Millimeters(0.),
                outer_only: false,
                open_start: false,
                behaviour: JointBehaviour::LaserOff,
                slow_speed: None,
                repierce: false,
            }),
            bridges: Some(Bridges {
                width: Millimeters(1.),
                connections: vec![
                    Bridge { first: pick(0), second: pick(1) },
                    Bridge { first: pick(0), second: pick(2) },
                ],
            }),
            start: Start { spots: vec![spot(2)], ..Start::default() },
            order: CutOrder {
                strategy: OrderStrategy::Manual(vec![2, 1, 0]),
                ..CutOrder::default()
            },
            ..Features::default()
        };
        let renumbered = features.renumbered_locations(|contour| match contour {
            0 => Some(0),
            2 => Some(1),
            _ => None,
        });
        let Some(JointPlacement::Manual(joints)) = renumbered.joints.map(|j| j.placement) else {
            panic!("the joints stay manual");
        };
        assert_eq!(joints.iter().map(|s| s.contour).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(renumbered.bridges, features.bridges);
        assert_eq!(renumbered.start.spots, vec![Spot { contour: 1, fraction: 0.5 }]);
        assert_eq!(renumbered.order.strategy, OrderStrategy::Manual(vec![1, 0]));
    }
}
