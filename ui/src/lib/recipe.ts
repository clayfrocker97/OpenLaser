// The recipe as the operator edits it: one semantic control per family of
// the vendor's attributes, on three pages. Cutting is the six controls
// along the path; Piercing is one mode and its stages in the order they
// run; Process options are the optional behaviours, each a card with a
// sheet. What the machine cannot do stays out of the form, and every
// attribute the file holds is kept and shown in the imported-values list.

import type { CapabilitiesView, LaserMode } from '../api';
import policy from '../api/bindings/recipe-policy.json';
import { displayNumber, unitLabel } from './units.svelte';
import { plural } from './format';

/** The recipe under edit, as the pages and sheets take it. */
export interface Editor {
  /** The file's values with the staged edits over them. */
  values: Values;
  edits: Values;
  /** The key the keypad is typing into. */
  editing: string | null;
  laser: LaserMode;
  a: Available;
  set: (key: string, value: string) => void;
  setMany: (edits: Values) => void;
  /** Opens the control for a key: the keypad, the keyboard, or a flip. */
  tap: (key: string, override?: { value: string; commit: (value: string) => void }) => void;
}

export type Kind = 'number' | 'flag' | 'gas' | 'text';
export interface Field {
  label: string;
  unit?: string;
  kind: Kind;
  /** The lowest value; negative when the field takes a sign. */
  min?: number;
  max?: number;
  /** Whole numbers only. */
  whole?: boolean;
  /** A line of help for the keypad. */
  help?: string;
  /** What it does, in a line under the label. */
  explain?: string;
}

/** The controller's six gas selections, as the machine files number them. */
export const GAS = ['Low Air', 'Low O₂', 'Low N₂', 'High Air', 'High O₂', 'High N₂'];
/** A gas selector's name, or its number when the card has more. */
export const gasName = (selector: number | string): string => GAS[Number(selector)] ?? `Selection ${selector}`;
/** The `ManuType` for each piercing stage count under a following head. */
export const LEVELS = [0, 2, 3, 4, 6, 7];
/** The piercing stages each `ManuType` runs. */
export const STAGES = policy.stages;
/** The three names of one stage's duration slot, canonical first. */
export const DURATION_ALIASES = policy.duration.aliases;

const n = (label: string, unit: string, min = 0, max = 100000, help?: string): Field => ({ label, unit, kind: 'number', min, max, help });
const whole = (label: string, unit: string, min = 0, max = 100000, help?: string): Field => ({ label, unit, kind: 'number', min, max, whole: true, help });
const flag = (label: string): Field => ({ label, kind: 'flag' });
const gas = (label: string): Field => ({ label, kind: 'gas' });
const text = (label: string): Field => ({ label, kind: 'text' });
const percent = (label: string, help?: string) => whole(label, '%', 0, 100, help);
const hertz = (label: string) => whole(label, 'Hz', 1, 65535);
const ms = (label: string, help?: string) => whole(label, policy.duration.unit, policy.duration.min, policy.duration.max, help);

// Labels are M-Laser's own, so a recipe reads the same in both; the common
// ones carry a line underneath that says what they do (ui/DESIGN.md,
// "Power and duty"). Cut Power is the PWM duty; Peak Current is the
// laser's output while the beam is on.
const PEAK = 'Laser output while the beam is on, in % of its rating.';
const PWM = 'PWM duty: the share of each pulse the beam is on.';
/** A field with a line under its label. */
const says = (f: Field, explain: string): Field => ({ ...f, explain });

const FIELDS: Record<string, Field> = {
  OpenLaserNozzleDiameter: n('Nozzle diameter', 'mm', 0.01, 20, 'Diameter of the nozzle to fit for this recipe.'),
  OpenLaserNozzleType: text('Nozzle type'),
  OpenLaserManualFocus: n('Focus', 'mm', -1000, 1000, 'Manual focus offset: set it at the head. It does not move Z or change the cut height.'),
  OpenLaserLens: n('Lens', 'mm', 1, 2000, 'Focal length of the focusing lens this recipe was made with, such as 150 mm.'),
  CutSpeed: says(n('Cut Speed', 'mm/s', 0.01, 100000), 'Speed along the cutting path.'),
  CutHeight: says(n('Cut Height', 'mm', 0, 1000), 'Nozzle gap above the sheet while cutting.'),
  CutPower: says(percent('Cut Power'), PWM),
  CutDuty: says(percent('Cut Power'), PWM),
  CutFreq: says(hertz('Cut Freq'), 'PWM pulses per second.'),
  CutPeakCurrent: says(percent('Peak Current'), PEAK),
  CutGasType: gas('Gas Type'),
  CutAirPressure: says(n('Gas Pressure', 'bar', 0, 100), 'Setpoint of the pressure regulator.'),
  AdvFixHeightCutPos: n('Absolute cut height', 'mm', 0, 1000),
  LaserOnDelay: says(ms('Laser On Delay'), 'Wait after the beam turns on, before moving.'),
  LaserOffBeforeDelay: says(ms('Before Laser Off Delay'), 'Wait at the end of a cut with the beam still on.'),
  LaserOffAfterDelay: ms('After Laser Off Delay'),
  UpHeight: says(n('Up Height', 'mm', 0, 1000), 'Height the head lifts to between contours.'),
  NoFollow: says(flag('Unfollow'), 'On: the head does not follow the sheet surface.'),
  ShortDistNoUp: says(flag('Short Unlift'), 'Stay down on short moves between contours.'),
  ShortDistGasKeepOn: flag('Short Keep Gas On'),
  NoCloseGasInManu: flag('Keep Gas On'),
  NoManu: says(flag('Uncut'), 'On: this layer is not cut.'),
  UD_UpEnable: says(flag('Start work Segment'), 'A slower first stretch of each cut.'), UD_UpAdvEnable: flag('Enable Laser Control'), UD_UpLen: n('Length', 'mm'), UD_UpSpeed: n('Work Speed', 'mm/s', 0.01), UD_UpDuty: percent('Laser Power [Duty]'), UD_UpFreq: hertz('Laser frequency'),
  UD_DownEnable: says(flag('End work Segment'), 'A slower last stretch of each cut.'), UD_DownAdvEnable: flag('Enable Laser Control'), UD_DownLen: n('Length', 'mm'), UD_DownSpeed: n('Work Speed', 'mm/s', 0.01), UD_DownDuty: percent('Laser Power [Duty]'), UD_DownFreq: hertz('Laser frequency'),
  SlowStart: flag('Slow start (legacy)'), SlowStartLength: n('Slow start length', 'mm'), SlowStartSpeed: n('Slow start speed', 'mm/s', 0.01),
  DrillHeight: says(n('Drill Height', 'mm', 0, 1000), 'Nozzle height for this piercing stage.'),
  DrillDelay: says(ms('Drill Time'), 'How long this stage pierces.'),
  GradualTime: ms('Gradual Drill Time'), FocusGradualTime: ms('Focus Gradual Time'),
  DrillPower: says(percent('Drill Power'), PWM), DrillGasPressure: n('Drill Gas Pressure', 'bar', 0, 100), DrillFreq: hertz('Drill Frequency'), DrillPeakCurrent: says(percent('Drill Peak Current'), PEAK), DrillGasType: gas('Drill Gas Type'),
  EnableGradualDrill: says(flag('Enable Gradual Drill'), 'Lower the head while piercing.'),
  BoltDrill_Enable: flag('Bolt Drill'), BoltDrill_Power: percent('Laser Power Duty [End]'), BoltDrill_Freq: hertz('Laser frequency [End]'),
  BeforeLaserOffDelay: ms('Before Laser Off Delay'), AfterLaserOffDelay: ms('After Laser Off Delay'),
  EnableSmoothPierce: flag('Smooth pierce'),
  SmoothPierceDrillHeight: n('Height', 'mm', 0, 1000), SmoothPierceDrillPower: says(percent('Duty cycle'), PWM), SmoothPierceDrillFreq: hertz('Frequency'), SmoothPierceDrillPeakCurrent: says(percent('Power'), PEAK),
  SmoothPierceDrillTime_ms: ms('Time'),
  PreDrill: says(flag('PreDrill'), 'Pierce every contour first, then cut.'), AfterPreDrillMustDrillBeforeCut: flag('Enable Before Cut Drill'), PreDrillIsNotUp: flag('Enable PreDrill No Up'),
  WithFilm: says(flag('With Film'), 'Remove protective film before each cut.'),
  EnableContourShift: flag('Enable Graph Shift'), ContourShiftXDist: n('Graph Shift X Direction Length', 'mm', -100000), ContourShiftYDist: n('Graph Shift Y Direction Length', 'mm', -100000),
  CleanResidue_Enable: flag('Clean Residue'), CleanResidue_WorkH: n('Work height', 'mm', 0, 1000), CleanResidue_WorkV: n('Work Speed', 'mm/s', 0.01), CleanResidue_GasType: gas('Gas type'), CleanResidue_GasP: n('Gas Pressure', 'bar', 0, 100),
  CleanResidue_PeakCurrent: says(percent('percent of Laser peakpower (elec Current)'), PEAK), CleanResidue_Power: says(percent('Laser Power [Duty]'), PWM), CleanResidue_Freq: hertz('Laser frequency'),
  CleanResidue_WorkR: n('Work radius', 'mm'), CleanResidue_SpiralTimes: whole('Coil cycle number', '', 1, 4096),
  PowerAdjustWithSpeed: says(flag('Dynamic Power'), 'Lower Cut Power as the head slows into corners.'), FreqAdjustWithSpeed: flag('Adjust frequency with speed'), PWMCurveNodes: text('Duty curve'),
  FreqCurveNodes: text('Frequency curve'), PowerCurveSmoothType: whole('Duty graph style', '', 0, 10),
  FreqCurveSmoothType: whole('Frequency graph style', '', 0, 10),
  ZFVibAbatType: whole('Vibration suppression', '', 0, 3), ZFVibAbat_Level: whole('Abatement factor - [thin]', '', 0, 255), ZFVibAbat_Level_Thick: whole('Abatement factor - [thick]', '', 0, 255),
  // Kept as the file holds them: optical focus this machine sets by hand, and switches nothing reads.
  CutFocusPos: n('Cut Focus Position', 'mm', -1000, 1000), DrillFocusPos: n('Drill Focus Position', 'mm', -1000, 1000), EnableFocusGradual: flag('Enable Focus Gradual'), FocusGradualEndPos: n('Focus Gradual End Position', 'mm', -1000, 1000),
  SmoothPierceDrillFocusPos: n('Focus', 'mm', -1000, 1000), CleanResidue_WorkFocus: n('Laser focus', 'mm', -1000, 1000),
  PreLaserOnFactor: n('Pre laser-on factor', ''), ZFVibAbat_Coef: n('Coefficient', ''), LeadLineParam_Enable: flag('Lead process (legacy)'), DrillTime: ms('Piercing time (unused)'),
  LayerFileName: text('Layer name'), ManuType: whole('Drill Type', '', 0, 7), Note: text('Notes'),
};

const STAGE_FIELDS = [
  'DrillHeight', 'DrillPower', 'DrillFreq', 'DrillPeakCurrent', 'DrillGasType', 'DrillGasPressure', 'DrillDelay', 'DrillFocusPos', 'DrillTime',
  'EnableGradualDrill', 'GradualTime', 'EnableFocusGradual', 'FocusGradualEndPos', 'FocusGradualTime', 'BeforeLaserOffDelay', 'AfterLaserOffDelay',
];
const STAGE_KEY = new RegExp(`^(${STAGE_FIELDS.join('|')})(\\d+)$|^BoltDrill_(Enable|Power|Freq)_(\\d+)$`);
/** The keys this machine cannot act on: manual optical focus, and the switches nothing reads. */
const UNAVAILABLE = new Set(policy.stored_fields);
const UNAVAILABLE_STAGE = new Set(policy.stored_stage_fields);

/** What a key is; a stage's key reads as its base field. */
export function field(key: string): Field {
  const known = FIELDS[key];
  if (known) return known;
  const m = STAGE_KEY.exec(key);
  const base = m ? (m[1] ?? `BoltDrill_${m[3]}`) : /^(.*?)_?(\d+)$/.exec(key)?.[1];
  return (base ? FIELDS[base] : undefined) ?? { label: key, kind: 'text' };
}

/** The bank a stage key belongs to, zero-based, or none. */
export function stageOf(key: string): number | null {
  const m = STAGE_KEY.exec(key);
  if (!m) return null;
  return m[2] !== undefined ? Number(m[2]) : Number(m[4]) - 1;
}

/** A stage key for bank `bank`: ramp keys count from one. */
export const stageKey = (base: string, bank: number): string => (base.startsWith('BoltDrill_') ? `${base}_${bank + 1}` : `${base}${bank}`);

/** Whether a value is a number, so the keypad edits it. */
export const numeric = (text: string): boolean => text.trim() !== '' && Number.isFinite(Number(text));
/** Whether a flag reads as on. */
export const isOn = (value: string | undefined): boolean => value === '1';
/** A flag the other way. */
export const toggled = (value: string): string => (isOn(value) ? '0' : '1');
/** A number as the file stores it. */
export const num = (value: number): string => String(Math.round(value * 1e6) / 1e6);

/** A value without its unit: the gas name, on or off, or the number to two decimals. */
export function bare(key: string, value: string): string {
  const f = field(key);
  if (value === '') return '—';
  if (f.kind === 'gas') return gasName(value);
  if (f.kind === 'flag') return isOn(value) ? 'On' : 'Off';
  return numeric(value) ? displayNumber(value, f.unit ?? '').replace(/(\.\d*?)0+$/, '$1').replace(/\.$/, '') : value;
}

/** A value as the operator reads it, with its unit. */
export function shown(key: string, value: string): string {
  const unit = unitLabel(field(key).unit ?? '');
  return unit && numeric(value) ? `${bare(key, value)} ${unit}` : bare(key, value);
}

/** Why a number is not accepted for `key`, or nothing. */
export function refused(key: string, value: number): string | null {
  const f = field(key);
  if (!Number.isFinite(value)) return 'Enter a number.';
  const min = f.min ?? 0;
  const max = f.max ?? 100000;
  if (value < min || value > max) return `Enter ${displayNumber(min, f.unit ?? '')} to ${displayNumber(max, f.unit ?? '')}${f.unit ? ` ${unitLabel(f.unit)}` : ''}.`;
  if (f.whole && !Number.isInteger(value)) return `Enter ${min} to ${max} as a whole number.`;
  return null;
}

/** A curve's nodes, `v,p,v,p…` in percent, as points. */
export const curvePoints = (nodes: string): Array<[number, number]> => {
  const v = nodes.split(',').map(Number);
  const points: Array<[number, number]> = [];
  for (let i = 0; i + 1 < v.length; i += 2) if (Number.isFinite(v[i]) && Number.isFinite(v[i + 1])) points.push([v[i]!, v[i + 1]!]);
  return points;
};

/** The gas selection a pressure field sets the pressure of. */
export function pairedGas(key: string): string | null {
  if (key === 'CutAirPressure') return 'CutGasType';
  if (key === 'CleanResidue_GasP') return 'CleanResidue_GasType';
  const stage = /^DrillGasPressure(\d+)$/.exec(key);
  return stage ? `DrillGasType${stage[1]}` : null;
}

// Piercing --------------------------------------------------------------------

export type Values = Record<string, string>;
export type Mode = 'none' | 'staged' | 'smooth';
export type HeadMode = 'follow' | 'fixed' | 'absolute';

/** The head's mode from `ManuType`: fixed height, absolute height, or following. */
export const headMode = (v: Values): HeadMode => (v['ManuType'] === '1' ? 'fixed' : v['ManuType'] === '5' ? 'absolute' : 'follow');
/** The ordinary stages the machining type runs. */
export const stageCount = (v: Values): number => STAGES[Number(v['ManuType'] ?? 0)] ?? 0;
/** The piercing mode: smooth replaces the stages, none means the cut starts directly. */
export const mode = (v: Values): Mode => (isOn(v['EnableSmoothPierce']) ? 'smooth' : stageCount(v) > 0 ? 'staged' : 'none');
/** The mode as the page names it. */
export const modeName = (v: Values): string => {
  const m = mode(v);
  return m === 'smooth' ? 'Smooth piercing' : m === 'staged' ? plural(stageCount(v), 'piercing stage') : 'No piercing';
};
/** The native bank of the visible stage: stages run from the highest bank down. */
export const bankOf = (count: number, visible: number): number => count - 1 - visible;
/** The `ManuType` for a head mode and stage count; a fixed or absolute head has no stages. */
export const manuType = (head: HeadMode, stages: number): string => (head === 'fixed' ? '1' : head === 'absolute' ? '5' : String(LEVELS[Math.max(0, Math.min(5, stages))]));

/** The edits that put the recipe in a piercing mode. */
export function modeEdits(v: Values, to: Mode, stages: number): Values {
  const edits: Values = {};
  if (to === 'smooth') edits['EnableSmoothPierce'] = '1';
  else if ('EnableSmoothPierce' in v || isOn(v['EnableSmoothPierce'])) edits['EnableSmoothPierce'] = '0';
  if (to === 'staged') edits['ManuType'] = manuType('follow', Math.max(1, stages));
  else if (to === 'none') edits['ManuType'] = manuType(headMode(v), 0);
  return edits;
}

/** A stage's duration from whichever of its three names are present: the
 * value when they agree, a conflict when they do not, blank when absent. */
export function duration(v: Values, bank: number): { value: string; conflict: boolean; present: string[] } {
  const present = DURATION_ALIASES.map((a) => `${a}${bank}`).filter((k) => k in v);
  const numbers = present.map((k) => Number(v[k]));
  const first = numbers[0];
  return { value: present.length ? v[present[0]!]! : '', conflict: numbers.some((x) => x !== first), present };
}

/** One duration edit for every present alias and the canonical name. */
export function durationEdits(v: Values, bank: number, value: string): Values {
  const edits: Values = { [`DrillDelay${bank}`]: value };
  for (const alias of DURATION_ALIASES) if (`${alias}${bank}` in v) edits[`${alias}${bank}`] = value;
  return edits;
}

/** Whether the file holds the bank's values at all. */
export const bankHeld = (v: Values, bank: number): boolean => `DrillHeight${bank}` in v;

// Cut start ---------------------------------------------------------------------

/** The cut start as the compiler takes it: the edge region first, the legacy slow start when that is off. */
export function cutStart(v: Values): { enabled: boolean; legacy: boolean; length: string; speed: string } {
  if (isOn(v['UD_UpEnable'])) return { enabled: true, legacy: false, length: v['UD_UpLen'] ?? '', speed: v['UD_UpSpeed'] ?? '' };
  if (isOn(v['SlowStart'])) return { enabled: true, legacy: true, length: v['SlowStartLength'] ?? '', speed: v['SlowStartSpeed'] ?? '' };
  return { enabled: false, legacy: false, length: v['UD_UpLen'] ?? v['SlowStartLength'] ?? '', speed: v['UD_UpSpeed'] ?? v['SlowStartSpeed'] ?? '' };
}

/** The edits that set the cut start as one control: the edge region carries
 * the effective values and the legacy slow start goes off with it. */
export function cutStartEdits(v: Values, change: { enabled?: boolean; length?: string; speed?: string }): Values {
  const now = cutStart(v);
  const edits: Values = { UD_UpEnable: change.enabled ?? now.enabled ? '1' : '0' };
  if ('SlowStart' in v || isOn(v['SlowStart'])) edits['SlowStart'] = '0';
  const length = change.length ?? now.length;
  const speed = change.speed ?? now.speed;
  if (length !== '') edits['UD_UpLen'] = length;
  if (speed !== '') edits['UD_UpSpeed'] = speed;
  return edits;
}

// Availability ----------------------------------------------------------------

/** What the recipe editor may offer, Machine backup. */
export interface Available {
  /** A wired assist-gas valve. CO2 always uses High Air. */
  gas: boolean;
  /** Whether a selection sets its pressure electronically. */
  pressure: (selector: number) => boolean;
  /** Whether the laser takes a power (peak power) setting. */
  peak: boolean;
  /** Whether the head's height is controlled. */
  height: boolean;
  /** The wired gas selections. */
  gases: number[];
  /** Fixed gas selection for processes without a gas chooser. */
  fixedGas: number | null;
  caps: CapabilitiesView | null;
}

/** Availability for `laser`; without machine files everything is offered. */
export function available(caps: CapabilitiesView[] | undefined, laser: LaserMode): Available {
  const c = caps?.find((x) => x.laser === laser) ?? null;
  const fiber = laser === 'fiber';
  return {
    gas: c ? c.gases.some((g) => g.valve && (fiber || g.selector === 3)) : true,
    pressure: (selector) => fiber && (c ? (c.gases.find((g) => g.selector === selector)?.pressure ?? false) : true),
    peak: c ? c.peak_output : true,
    height: c ? c.height_control : true,
    gases: c ? c.gases.filter((g) => g.valve && (fiber || g.selector === 3)).map((g) => g.selector) : fiber ? [0, 1, 2, 3, 4, 5] : [3],
    fixedGas: fiber ? null : 3,
    caps: c,
  };
}

// The imported values -------------------------------------------------------------

export type Status = 'active' | 'inactive' | 'unavailable' | 'unresolved';

/** What the recipe's process does with each attribute it holds. */
export function status(key: string, v: Values, a: Available): Status {
  const base = /^(.*?)_?\d+$/.exec(key)?.[1] ?? key;
  const stage = stageOf(key);
  if (!FIELDS[key] && !(stage !== null && FIELDS[base])) return 'unresolved';
  if (key.startsWith('OpenLaser') || UNAVAILABLE.has(key) || (stage !== null && UNAVAILABLE_STAGE.has(base))) return 'unavailable';
  if (a.fixedGas !== null && (key === 'CutGasType' || key === 'CutAirPressure' || base === 'DrillGasType' || base === 'DrillGasPressure' || key === 'CleanResidue_GasType' || key === 'CleanResidue_GasP')) return 'inactive';
  if ((key === 'CutGasType' || base === 'DrillGasType' || key === 'CleanResidue_GasType') && a.gas && !a.gases.includes(Number(v[key]))) return 'unresolved';
  const m = mode(v);
  if (stage !== null) return m === 'staged' && stage < stageCount(v) ? 'active' : 'inactive';
  if (key.startsWith('SmoothPierce')) return m === 'smooth' ? 'active' : 'inactive';
  if (key.startsWith('CleanResidue_') && key !== 'CleanResidue_Enable') return isOn(v['CleanResidue_Enable']) && m === 'staged' ? 'active' : 'inactive';
  if (key.startsWith('UD_Up') && key !== 'UD_UpEnable') return isOn(v['UD_UpEnable']) ? 'active' : 'inactive';
  if (key.startsWith('UD_Down') && key !== 'UD_DownEnable') return isOn(v['UD_DownEnable']) ? 'active' : 'inactive';
  if (key.startsWith('SlowStart')) return !isOn(v['UD_UpEnable']) && isOn(v['SlowStart']) ? 'active' : 'inactive';
  if (key === 'AfterPreDrillMustDrillBeforeCut' || key === 'PreDrillIsNotUp') return isOn(v['PreDrill']) && m === 'staged' ? 'active' : 'inactive';
  if (key === 'PreDrill') return m === 'staged' || !isOn(v[key]) ? 'active' : 'inactive';
  if (key.startsWith('ContourShift')) return isOn(v['EnableContourShift']) ? 'active' : 'inactive';
  if (key === 'PWMCurveNodes') return isOn(v['PowerAdjustWithSpeed']) ? 'active' : 'inactive';
  if (key === 'FreqCurveNodes') return isOn(v['FreqAdjustWithSpeed']) ? 'active' : 'inactive';
  if (key === 'AdvFixHeightCutPos') return headMode(v) === 'absolute' ? 'active' : 'inactive';
  if ((key === 'CutAirPressure' || key === 'CutGasType' || key === 'ShortDistGasKeepOn' || key === 'NoCloseGasInManu') && !a.gas) return 'unavailable';
  if (key === 'CutPeakCurrent' && !a.peak) return 'unavailable';
  if ((key === 'CutHeight' || key === 'UpHeight' || key === 'NoFollow' || key === 'ShortDistNoUp') && !a.height) return 'unavailable';
  return 'active';
}
