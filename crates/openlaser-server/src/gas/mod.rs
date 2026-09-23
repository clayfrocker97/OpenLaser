// SPDX-License-Identifier: GPL-3.0-or-later

//! Assist gas and laser time per job: what a compiled program will spend,
//! what each run actually spent, and what that costs at the shop's prices.
//!
//! Times come from the compiled program ([`usage`]); litres from the nozzle
//! flow model ([`flow`]) or a flow the operator entered, scaled by the
//! gas's calibration; money from the prices under Settings → Gas costs.
//! Prices and runs are saved in the data directory, apart from the library,
//! so they never enter the library's undo history.

pub mod flow;
pub mod runs;
pub mod usage;

use crate::{Error, Result};
use openlaser_core::units::{Bar, CubicMeters, Liters, LitersPerMinute, Millimeters, Seconds};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use runs::{RunOutcome, RunRecord};
pub use usage::{GasTime, JobUsage, Usage};

/// An amount of money in the shop's currency.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Money(pub f64);

/// An assist gas, whatever valve or pressure route carries it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GasKind {
    /// Nitrogen.
    Nitrogen,
    /// Oxygen.
    Oxygen,
    /// Compressed air.
    Air,
}

impl GasKind {
    /// Every gas, in the order the settings list them.
    pub const ALL: [Self; 3] = [Self::Nitrogen, Self::Oxygen, Self::Air];

    /// The gas a valve selector carries: selectors 0 to 5 are low air, low
    /// oxygen, low nitrogen, high air, high oxygen and high nitrogen.
    #[must_use]
    pub const fn of_selector(selector: u8) -> Self {
        match selector % 3 {
            0 => Self::Air,
            1 => Self::Oxygen,
            _ => Self::Nitrogen,
        }
    }
}

/// How a nozzle is built.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NozzleType {
    /// One bore.
    #[default]
    Single,
    /// An inner and an outer layer.
    Double,
}

/// The nozzle a recipe fits.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Nozzle {
    /// Exit diameter.
    pub diameter: Millimeters,
    /// Single or double layer.
    pub kind: NozzleType,
}

impl Nozzle {
    /// The nozzle a recipe's attributes name, when it names a diameter.
    #[must_use]
    pub fn of_attributes(attributes: &BTreeMap<String, String>) -> Option<Self> {
        let diameter = attributes
            .get("OpenLaserNozzleDiameter")
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d > 0. && *d <= 20.)?;
        let kind = match attributes.get("OpenLaserNozzleType").map(|v| v.trim()) {
            Some("double") => NozzleType::Double,
            _ => NozzleType::Single,
        };
        Some(Self { diameter: Millimeters(diameter), kind })
    }

    fn validate(&self) -> Result<()> {
        if !(self.diameter.0.is_finite() && self.diameter.0 > 0. && self.diameter.0 <= 20.) {
            return Err(Error::Request(
                "a nozzle diameter must be above 0 and at most 20 mm".into(),
            ));
        }
        Ok(())
    }
}

/// Where a gas comes from and what it costs.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Cylinders, paid per refill of a known gas content.
    Refill {
        /// The price of one refill; zero until the shop enters it.
        price: Money,
        /// The gas one cylinder holds at standard conditions.
        volume: CubicMeters,
    },
    /// Bulk or tank supply, paid per cubic metre.
    Bulk {
        /// The price per standard cubic metre.
        price_per_m3: Money,
    },
    /// A compressor, paid for the hours the valve is open. Air only.
    Compressor {
        /// Running cost per hour.
        cost_per_hour: Money,
    },
}

/// How the flow of a gas is known.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Flow {
    /// From the recipe's nozzle and pressure ([`flow::estimate`]), times the
    /// gas's calibration factor.
    #[default]
    Estimated,
    /// A fixed flow the operator measured, whatever the nozzle.
    Manual {
        /// The flow.
        rate: LitersPerMinute,
    },
}

/// A saved 60-second flow test: the measured over the estimated volume.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// Measured litres over estimated litres; multiplies every estimate.
    pub factor: f64,
    /// When it was measured, in seconds since the epoch.
    pub at: u64,
    /// The test pressure.
    pub pressure: Bar,
    /// The nozzle fitted for the test.
    pub nozzle: Nozzle,
    /// What the uncalibrated model expected over the test.
    pub estimated: Liters,
    /// What the operator measured.
    pub measured: Liters,
}

/// The shop's supply of one gas.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Supply {
    /// Where it comes from and its price.
    pub source: Source,
    /// How its flow is known.
    pub flow: Flow,
    /// The latest flow test, if one was saved.
    pub calibration: Option<Calibration>,
}

impl Supply {
    const fn cylinders() -> Self {
        Self {
            source: Source::Refill { price: Money(0.), volume: CubicMeters(10.) },
            flow: Flow::Estimated,
            calibration: None,
        }
    }

    /// The price of a standard cubic metre, once one is entered.
    #[must_use]
    pub fn price_per_m3(&self) -> Option<Money> {
        match self.source {
            Source::Refill { price, volume } if price.0 > 0. && volume.0 > 0. => {
                Some(Money(price.0 / volume.0))
            }
            Source::Bulk { price_per_m3 } if price_per_m3.0 > 0. => Some(price_per_m3),
            _ => None,
        }
    }

    /// The flow of this gas through `nozzle` at `pressure`, if it is known.
    #[must_use]
    pub fn rate(
        &self,
        gas: GasKind,
        nozzle: Option<Nozzle>,
        pressure: Bar,
    ) -> Option<LitersPerMinute> {
        match self.flow {
            Flow::Manual { rate } => Some(rate),
            Flow::Estimated => nozzle.map(|nozzle| {
                let factor = self.calibration.map_or(1., |c| c.factor);
                LitersPerMinute(flow::estimate(gas, nozzle, pressure).0 * factor)
            }),
        }
    }

    /// What `litres` over `seconds` of open valve costs, once priced.
    #[must_use]
    pub fn cost(&self, litres: Option<Liters>, seconds: Seconds) -> Option<Money> {
        match self.source {
            Source::Compressor { cost_per_hour } if cost_per_hour.0 > 0. => {
                Some(Money(seconds.0 / 3600. * cost_per_hour.0))
            }
            Source::Compressor { .. } => None,
            _ => Some(Money(litres?.0 / 1000. * self.price_per_m3()?.0)),
        }
    }

    fn validate(&self, gas: GasKind) -> Result<()> {
        let money = |m: Money| m.0.is_finite() && (0. ..=1e7).contains(&m.0);
        let valid = match self.source {
            Source::Refill { price, volume } => {
                money(price) && volume.0.is_finite() && volume.0 > 0. && volume.0 <= 10_000.
            }
            Source::Bulk { price_per_m3 } => money(price_per_m3),
            Source::Compressor { cost_per_hour } => {
                if gas != GasKind::Air {
                    return Err(Error::Request("only air can come from a compressor".into()));
                }
                money(cost_per_hour)
            }
        };
        if !valid {
            return Err(Error::Request(
                "prices must be 0 or more and a cylinder must hold more than 0 m³".into(),
            ));
        }
        if let Flow::Manual { rate } = self.flow
            && !(rate.0.is_finite() && rate.0 > 0. && rate.0 <= 100_000.)
        {
            return Err(Error::Request("a manual flow must be above 0 L/min".into()));
        }
        if let Some(calibration) = &self.calibration {
            check_factor(calibration.factor)?;
        }
        Ok(())
    }
}

/// The lowest and highest calibration factor accepted: a test outside
/// this range is a measuring mistake, not a nozzle.
pub const FACTOR_RANGE: (f64, f64) = (0.2, 5.);

fn check_factor(factor: f64) -> Result<()> {
    if !(factor.is_finite() && (FACTOR_RANGE.0..=FACTOR_RANGE.1).contains(&factor)) {
        return Err(Error::Request(format!(
            "the measured volume is {factor:.2} times the estimate; a test between {} and {} times is expected, so measure again",
            FACTOR_RANGE.0, FACTOR_RANGE.1
        )));
    }
    Ok(())
}

/// Settings → Gas costs: the currency and every gas's supply.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GasCosts {
    /// The currency symbol money is shown with.
    pub currency: String,
    /// Nitrogen.
    pub nitrogen: Supply,
    /// Oxygen.
    pub oxygen: Supply,
    /// Air.
    pub air: Supply,
}

impl Default for GasCosts {
    fn default() -> Self {
        Self {
            currency: "$".into(),
            nitrogen: Supply::cylinders(),
            oxygen: Supply::cylinders(),
            air: Supply {
                source: Source::Compressor { cost_per_hour: Money(0.) },
                flow: Flow::Estimated,
                calibration: None,
            },
        }
    }
}

impl GasCosts {
    /// The supply of `gas`.
    #[must_use]
    pub const fn supply(&self, gas: GasKind) -> &Supply {
        match gas {
            GasKind::Nitrogen => &self.nitrogen,
            GasKind::Oxygen => &self.oxygen,
            GasKind::Air => &self.air,
        }
    }

    const fn supply_mut(&mut self, gas: GasKind) -> &mut Supply {
        match gas {
            GasKind::Nitrogen => &mut self.nitrogen,
            GasKind::Oxygen => &mut self.oxygen,
            GasKind::Air => &mut self.air,
        }
    }

    /// Every price and flow within range, and a short currency symbol.
    pub fn validate(&self) -> Result<()> {
        let symbol = self.currency.trim();
        if symbol.is_empty() || symbol.chars().count() > 4 || symbol.chars().any(char::is_control) {
            return Err(Error::Request("the currency symbol must be 1 to 4 characters".into()));
        }
        for gas in GasKind::ALL {
            self.supply(gas).validate(gas)?;
        }
        Ok(())
    }

    /// `usage` in litres and money with `nozzle` fitted.
    #[must_use]
    pub fn price(&self, usage: &Usage, nozzle: Option<Nozzle>) -> Consumption {
        let gases: Vec<GasLine> = usage
            .gases
            .iter()
            .map(|time| {
                let supply = self.supply(time.gas);
                let flow = supply.rate(time.gas, nozzle, time.pressure);
                let litres = flow.map(|f| Liters(f.0 * time.seconds.0 / 60.));
                GasLine {
                    gas: time.gas,
                    pressure: time.pressure,
                    seconds: time.seconds,
                    flow,
                    litres,
                    cost: supply.cost(litres, time.seconds),
                }
            })
            .collect();
        let cost = gases.iter().try_fold(0., |sum, line| line.cost.map(|c| sum + c.0)).map(Money);
        Consumption {
            laser: usage.laser,
            pierces: usage.pierces,
            cut: usage.cut,
            nozzle,
            gases,
            cost,
        }
    }
}

/// One gas's share of a job or run.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GasLine {
    /// Which gas.
    pub gas: GasKind,
    /// Its gauge pressure.
    pub pressure: Bar,
    /// Valve-open time.
    pub seconds: Seconds,
    /// The flow used, when known.
    pub flow: Option<LitersPerMinute>,
    /// The volume, when the flow is known.
    pub litres: Option<Liters>,
    /// The cost, when priced.
    pub cost: Option<Money>,
}

/// A job's or run's laser time, gas and cost.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Consumption {
    /// Laser-on time.
    pub laser: Seconds,
    /// Pierces.
    pub pierces: u32,
    /// Length cut.
    pub cut: Millimeters,
    /// The nozzle the recipe fits, if it names one.
    pub nozzle: Option<Nozzle>,
    /// Gas by gas and pressure.
    pub gases: Vec<GasLine>,
    /// The total gas cost, when every gas used is priced.
    pub cost: Option<Money>,
}

/// One gas's derived figures for the settings page.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SupplyView {
    /// Which gas.
    pub gas: GasKind,
    /// The price per standard cubic metre, when priced by volume.
    pub price_per_m3: Option<Money>,
}

/// Gas costs as every screen sees them.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct GasView {
    /// Changes when prices or recorded runs change.
    pub revision: u64,
    /// The saved settings.
    pub costs: GasCosts,
    /// Derived prices, in [`GasKind::ALL`] order.
    pub supplies: Vec<SupplyView>,
    /// One run of the job being set up, once it is compiled.
    pub estimate: Option<Consumption>,
    /// Why saved prices or runs could not be read or written.
    pub error: Option<String>,
}

/// A request to price a nozzle and pressure, for the flow test.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowQuery {
    /// Which gas.
    pub gas: GasKind,
    /// The test pressure.
    pub pressure: Bar,
    /// The nozzle fitted.
    pub nozzle: Nozzle,
    /// How long the valve is open.
    pub seconds: Seconds,
}

/// What the model expects over a flow test.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FlowEstimate {
    /// Uncalibrated.
    pub estimated: Liters,
    /// With the gas's current calibration.
    pub calibrated: Liters,
}

/// A measured flow test to save as a gas's calibration.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationRequest {
    /// Which gas.
    pub gas: GasKind,
    /// The test pressure.
    pub pressure: Bar,
    /// The nozzle fitted.
    pub nozzle: Nozzle,
    /// How long the valve was open.
    pub seconds: Seconds,
    /// The litres the operator measured.
    pub measured: Liters,
}

impl CalibrationRequest {
    /// The flow test it reports.
    #[must_use]
    pub const fn test(&self) -> FlowQuery {
        FlowQuery {
            gas: self.gas,
            pressure: self.pressure,
            nozzle: self.nozzle,
            seconds: self.seconds,
        }
    }
}

/// The flow test the machine's timed gas action runs.
pub const CALIBRATION_DURATION: std::time::Duration = std::time::Duration::from_mins(1);

/// [`CALIBRATION_DURATION`] in seconds, the longest flow test priced.
pub const CALIBRATION_SECONDS: f64 = 60.;

const COSTS_FILE: &str = "gas-costs.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedCosts {
    version: u32,
    costs: GasCosts,
}

/// Where a run in progress came from, for its record.
#[derive(Clone, Debug)]
pub(crate) struct ActiveRun {
    pub execution: u64,
    pub record: String,
    pub key: String,
    pub job: Option<openlaser_library::Id>,
    pub name: String,
    pub nozzle: Option<Nozzle>,
    pub started: u64,
}

/// Saved gas prices, recorded runs and the run in progress.
pub struct Store {
    root: PathBuf,
    costs: GasCosts,
    runs: runs::Runs,
    revision: u64,
    error: Option<String>,
    pub(crate) active: Option<ActiveRun>,
    view: Arc<GasView>,
    stamp: Option<(u64, u64)>,
}

impl Store {
    /// Opens the saved prices and runs. A missing file means none yet; an
    /// unreadable one is reported and left untouched on disk.
    #[must_use]
    pub fn open(root: &Path) -> Self {
        let mut error = None;
        let path = root.join(COSTS_FILE);
        let costs = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<SavedCosts>(&bytes)
                .map_err(|e| e.to_string())
                .and_then(|saved| {
                    if saved.version != 1 {
                        return Err(format!("unsupported version {}", saved.version));
                    }
                    saved.costs.validate().map_err(|e| e.to_string())?;
                    Ok(saved.costs)
                })
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, path = %path.display(), "gas costs unreadable");
                    error = Some(format!(
                        "Saved gas costs could not be read ({e}); saving replaces them."
                    ));
                    GasCosts::default()
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => GasCosts::default(),
            Err(e) => {
                error = Some(format!("Saved gas costs could not be read: {e}"));
                GasCosts::default()
            }
        };
        let runs = runs::Runs::open(root);
        if let Some(problem) = runs.error() {
            error.get_or_insert_with(|| problem.to_owned());
        }
        Self {
            root: root.to_path_buf(),
            costs,
            runs,
            revision: 1,
            error,
            active: None,
            view: Arc::default(),
            stamp: None,
        }
    }

    /// The saved prices.
    #[must_use]
    pub const fn costs(&self) -> &GasCosts {
        &self.costs
    }

    /// Replaces the prices, if nobody changed them since `expected` was read.
    pub fn save(&mut self, costs: GasCosts, expected: &GasCosts) -> Result<()> {
        costs.validate()?;
        if *expected != self.costs {
            return Err(Error::Refused(
                "gas costs changed on another screen; review them again".into(),
            ));
        }
        self.write(costs)
    }

    fn write(&mut self, mut costs: GasCosts) -> Result<()> {
        costs.currency = String::from(costs.currency.trim());
        let saved = SavedCosts { version: 1, costs };
        let bytes = serde_json::to_vec_pretty(&saved).map_err(|e| Error::Refused(e.to_string()))?;
        openlaser_library::atomic_write(&self.root.join(COSTS_FILE), &bytes)?;
        self.costs = saved.costs;
        if self.error.as_deref().is_some_and(|e| e.starts_with("Saved gas costs")) {
            self.error = None;
        }
        self.revision += 1;
        Ok(())
    }

    /// What the model expects over a flow test.
    pub fn flow(&self, query: &FlowQuery) -> Result<FlowEstimate> {
        query.nozzle.validate()?;
        let pressure =
            query.pressure.0.is_finite() && query.pressure.0 > 0. && query.pressure.0 <= 100.;
        let seconds = query.seconds.0 > 0. && query.seconds.0 <= CALIBRATION_SECONDS;
        if !pressure || !seconds {
            return Err(Error::Request("a flow test needs 0–100 bar and up to 60 s".into()));
        }
        let minutes = query.seconds.0 / 60.;
        let estimated = flow::estimate(query.gas, query.nozzle, query.pressure).0 * minutes;
        let factor = self.costs.supply(query.gas).calibration.map_or(1., |c| c.factor);
        Ok(FlowEstimate { estimated: Liters(estimated), calibrated: Liters(estimated * factor) })
    }

    /// Saves a measured flow test as the gas's calibration and returns it.
    pub fn calibrate(&mut self, request: &CalibrationRequest) -> Result<Calibration> {
        let test = request.test();
        let expected = self.flow(&test)?.estimated;
        if expected.0 <= 0. {
            return Err(Error::Request("the model expects no flow at this pressure".into()));
        }
        if !(request.measured.0.is_finite() && request.measured.0 > 0.) {
            return Err(Error::Request("enter the litres the test used".into()));
        }
        let factor = request.measured.0 / expected.0;
        check_factor(factor)?;
        let calibration = Calibration {
            factor,
            at: openlaser_library::now(),
            pressure: test.pressure,
            nozzle: test.nozzle,
            estimated: expected,
            measured: request.measured,
        };
        let mut costs = self.costs.clone();
        costs.supply_mut(test.gas).calibration = Some(calibration);
        self.write(costs)?;
        Ok(calibration)
    }

    /// Recorded runs of the job saved as `job` or authored as `key`, newest first.
    #[must_use]
    pub fn runs(&self, key: &str, job: Option<&str>) -> Vec<RunRecord> {
        self.runs.matching(key, job)
    }

    /// Records or updates the run of `active`, priced now.
    pub(crate) fn record(
        &mut self,
        active: &ActiveRun,
        outcome: RunOutcome,
        fraction: f64,
        usage: &Usage,
    ) {
        let record = RunRecord {
            id: active.record.clone(),
            key: active.key.clone(),
            job: active.job.clone(),
            name: active.name.clone(),
            started: active.started,
            finished: openlaser_library::now(),
            outcome,
            fraction: fraction.clamp(0., 1.),
            consumption: self.costs.price(usage, active.nozzle),
            currency: self.costs.currency.clone(),
        };
        if let Err(error) = self.runs.upsert(record) {
            tracing::warn!(%error, "gas run history not saved");
            self.error = Some(format!("The run history could not be saved: {error}"));
        }
        self.revision += 1;
    }

    /// The published view, rebuilt only when prices, runs or the draft change.
    pub(crate) fn view(
        &mut self,
        draft_revision: u64,
        draft: Option<&crate::document::DraftView>,
    ) -> Arc<GasView> {
        let stamp = (draft_revision, self.revision);
        if self.stamp != Some(stamp) {
            let estimate = draft.and_then(|d| {
                let compiled = d.compiled.as_ref()?;
                let nozzle = d.recipe.as_ref().and_then(|r| Nozzle::of_attributes(&r.attributes));
                Some(self.costs.price(&compiled.usage, nozzle))
            });
            self.view = Arc::new(GasView {
                revision: self.revision,
                costs: self.costs.clone(),
                supplies: GasKind::ALL
                    .iter()
                    .map(|gas| SupplyView {
                        gas: *gas,
                        price_per_m3: self.costs.supply(*gas).price_per_m3(),
                    })
                    .collect(),
                estimate,
                error: self.error.clone(),
            });
            self.stamp = Some(stamp);
        }
        self.view.clone()
    }
}

/// The executed share of each pass: completed passes whole, partial passes
/// by the union of their executed intervals, points once started.
#[must_use]
pub fn executed_shares(steps: &[crate::resume::RecoveryStep]) -> Vec<f64> {
    steps
        .iter()
        .map(|step| {
            if step.status == "completed" {
                return 1.;
            }
            if step.length_mm <= 0. {
                return if step.executed.is_empty() { 0. } else { 1. };
            }
            let mut ranges: Vec<[f64; 2]> = step
                .executed
                .iter()
                .map(|r| [r[0].clamp(0., 1.), r[1].clamp(0., 1.)])
                .filter(|r| r[1] > r[0])
                .collect();
            ranges.sort_by(|a, b| a[0].total_cmp(&b[0]));
            let (mut total, mut reach) = (0., 0_f64);
            for [from, to] in ranges {
                let from = from.max(reach);
                if to > from {
                    total += to - from;
                }
                reach = reach.max(to);
            }
            total.clamp(0., 1.)
        })
        .collect()
}

impl crate::Coordinator {
    /// Starts the record of a fresh job run from the draft it runs.
    pub(crate) fn begin_gas_run(&mut self, execution: u64) {
        self.gas.active = self.draft.as_ref().map(|draft| {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            ActiveRun {
                execution,
                record: format!("{nanos:x}-{execution}"),
                key: crate::workspace::key(draft),
                job: draft.job.clone(),
                name: self.draft_name(draft),
                nozzle: draft.recipe.as_ref().and_then(|r| Nozzle::of_attributes(&r.attributes)),
                started: openlaser_library::now(),
            }
        });
    }

    /// Records what the retained run has spent so far: all of it when it
    /// completed, otherwise the share of each pass its feedback confirmed.
    /// A resumed run updates the same record. Dry runs are not recorded.
    pub(crate) fn record_gas(&mut self, completed: bool) {
        // Only a job run holds its program; a frame or a pulse does not.
        if self.held.is_none() {
            return;
        }
        let Some(active) = self.gas.active.clone() else { return };
        let state = self.machine.state();
        let Some(recovery) = self.recovery.as_mut().filter(|r| r.execution.id == active.execution)
        else {
            return;
        };
        if recovery.original.dry_run {
            return;
        }
        recovery.observe(&state, self.execution.as_deref());
        let usage = recovery.execution.compiled.pass_usage.clone();
        let shares = if completed {
            vec![1.; usage.passes.len()]
        } else {
            executed_shares(&recovery.view().steps)
        };
        let executed = usage.executed(&shares);
        let total_cut: f64 = usage.passes.iter().map(|p| p.cut.0).sum();
        let fraction = if completed {
            1.
        } else if total_cut > 0. {
            executed.cut.0 / total_cut
        } else if shares.is_empty() {
            0.
        } else {
            shares.iter().sum::<f64>() / usage::count(shares.len())
        };
        let outcome = if completed { RunOutcome::Done } else { RunOutcome::Stopped };
        self.gas.record(&active, outcome, fraction, &executed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openlaser-gas-{name}-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&dir));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const NOZZLE: Nozzle = Nozzle { diameter: Millimeters(1.5), kind: NozzleType::Single };

    fn usage() -> Usage {
        Usage {
            laser: Seconds(100.),
            gases: vec![
                GasTime { gas: GasKind::Nitrogen, pressure: Bar(12.), seconds: Seconds(120.) },
                GasTime { gas: GasKind::Air, pressure: Bar(6.), seconds: Seconds(1800.) },
            ],
            pierces: 4,
            cut: Millimeters(1000.),
        }
    }

    #[test]
    fn refills_bulk_and_compressor_price_the_gas_used() {
        let mut costs = GasCosts::default();
        let unpriced = costs.price(&usage(), Some(NOZZLE));
        assert_eq!(unpriced.cost, None, "unpriced gases have no total");
        costs.nitrogen.source = Source::Refill { price: Money(90.), volume: CubicMeters(9.) };
        costs.air.source = Source::Compressor { cost_per_hour: Money(2.) };
        let priced = costs.price(&usage(), Some(NOZZLE));
        let n2 = priced.gases[0];
        let lpm = flow::estimate(GasKind::Nitrogen, NOZZLE, Bar(12.)).0;
        assert!((n2.litres.unwrap().0 - lpm * 2.).abs() < 1e-9);
        assert!((n2.cost.unwrap().0 - lpm * 2. / 1000. * 10.).abs() < 1e-9);
        assert!((priced.gases[1].cost.unwrap().0 - 1.).abs() < 1e-9, "half an hour at $2/h");
        assert!((priced.cost.unwrap().0 - (n2.cost.unwrap().0 + 1.)).abs() < 1e-9);

        costs.nitrogen.source = Source::Bulk { price_per_m3: Money(4.) };
        costs.nitrogen.flow = Flow::Manual { rate: LitersPerMinute(300.) };
        let manual = costs.price(&usage(), None);
        assert_eq!(manual.gases[0].litres, Some(Liters(600.)));
        assert!((manual.gases[0].cost.unwrap().0 - 2.4).abs() < 1e-9);
        assert_eq!(costs.price(&usage(), None).gases[1].litres, None, "no nozzle, no estimate");
    }

    #[test]
    fn calibration_scales_estimates_and_rejects_implausible_tests() {
        let root = scratch("calibrate");
        let mut store = Store::open(&root);
        let test = FlowQuery {
            gas: GasKind::Oxygen,
            pressure: Bar(1.),
            nozzle: NOZZLE,
            seconds: Seconds(CALIBRATION_SECONDS),
        };
        let expected = store.flow(&test).unwrap().estimated.0;
        assert!(expected > 0.);
        let request = |measured: f64| CalibrationRequest {
            gas: test.gas,
            pressure: test.pressure,
            nozzle: test.nozzle,
            seconds: test.seconds,
            measured: Liters(measured),
        };
        let saved = store.calibrate(&request(expected * 1.25)).unwrap();
        assert!((saved.factor - 1.25).abs() < 1e-9);
        assert!((store.flow(&test).unwrap().calibrated.0 - expected * 1.25).abs() < 1e-9);
        assert!(store.calibrate(&request(expected * 9.)).is_err());
        assert!(store.calibrate(&request(0.)).is_err());
        let reopened = Store::open(&root);
        assert_eq!(reopened.costs().oxygen.calibration, Some(saved));
        assert!(reopened.error.is_none());
    }

    #[test]
    fn saving_is_checked_against_the_version_read_and_bad_files_are_reported() {
        let root = scratch("save");
        let mut store = Store::open(&root);
        let before = store.costs().clone();
        let mut next = before.clone();
        next.currency = " € ".into();
        store.save(next.clone(), &before).unwrap();
        assert_eq!(store.costs().currency, "€");
        assert!(store.save(next.clone(), &before).is_err(), "stale edit refused");
        let mut bad = store.costs().clone();
        bad.oxygen.source = Source::Compressor { cost_per_hour: Money(1.) };
        assert!(store.save(bad, &store.costs().clone()).is_err());
        let mut long = store.costs().clone();
        long.currency = "EURO$".into();
        assert!(long.validate().is_err());
        std::fs::write(root.join(COSTS_FILE), b"{").unwrap();
        let damaged = Store::open(&root);
        assert_eq!(damaged.costs(), &GasCosts::default());
        assert!(damaged.error.is_some());
        assert_eq!(std::fs::read(root.join(COSTS_FILE)).unwrap(), b"{", "left untouched");
    }

    #[test]
    fn nozzles_are_read_from_recipe_attributes() {
        let mut attributes = BTreeMap::new();
        assert_eq!(Nozzle::of_attributes(&attributes), None);
        attributes.insert("OpenLaserNozzleDiameter".into(), "2.0".into());
        attributes.insert("OpenLaserNozzleType".into(), "double".into());
        assert_eq!(
            Nozzle::of_attributes(&attributes),
            Some(Nozzle { diameter: Millimeters(2.), kind: NozzleType::Double })
        );
        attributes.insert("OpenLaserNozzleDiameter".into(), "abc".into());
        assert_eq!(Nozzle::of_attributes(&attributes), None);
        assert_eq!(GasKind::of_selector(5), GasKind::Nitrogen);
        assert_eq!(GasKind::of_selector(1), GasKind::Oxygen);
        assert_eq!(GasKind::of_selector(3), GasKind::Air);
    }
}
