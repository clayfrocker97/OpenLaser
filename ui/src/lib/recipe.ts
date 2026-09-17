// The recipe as the operator edits it: one semantic control per family of
// the vendor's attributes, on three pages. Cutting is the six controls
// along the path; Piercing is one mode and its stages in the order they
// run; Process options are the optional behaviours, each a card with a
// sheet. What the machine cannot do stays out of the form, and every
// attribute the file holds is kept and shown in the imported-values list.

import type { CapabilitiesView, LaserMode } from '../api';
import policy from '../api/bindings/recipe-policy.json';
import { displayNumber, unitLabel } from './units.svelte';

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
}

/** The controller's six gas selections, as the machine files number them. */
export const GAS = ['Low Air', 'Low O₂', 'Low N₂', 'High Air', 'High O₂', 'High N₂'];
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

const FIELDS: Record<string, Field> = {
  OpenLaserNozzleDiameter: n('Nozzle diameter', 'mm', 0.01, 20, 'Diameter of the nozzle to fit for this recipe.'),
  OpenLaserNozzleType: text('Nozzle type'),
  OpenLaserManualFocus: n('Manual focus', 'mm', -1000, 1000, 'Set this optical focus at the head. This value does not move Z or change nozzle gap.'),
  CutSpeed: n('Cutting speed', 'mm/s', 0.01, 100000, 'Speed along the cutting path. Cut start and cut end can use their own slower regions.'),
  CutHeight: n('Nozzle gap', 'mm', 0, 1000, 'Distance between the nozzle and the sheet while following. Optical focus is set by hand.'),
  CutPower: percent('Duty cycle', 'The part of each pulse period the laser command is on.'),
  CutDuty: percent('Duty cycle', 'The part of each pulse period the laser command is on.'),
  CutFreq: hertz('Frequency'),
  CutPeakCurrent: percent('Peak output', 'The laser’s output-level command during the on part of a pulse, not measured watts.'),
  CutGasType: gas('Cutting gas'),
  CutAirPressure: n('Gas pressure', 'bar', 0, 100, 'The setpoint of the proportional regulator.'),
  AdvFixHeightCutPos: n('Absolute cut height', 'mm', 0, 1000),
  LaserOnDelay: ms('Cutting start dwell'),
  LaserOffBeforeDelay: ms('Cutting end dwell'),
  LaserOffAfterDelay: ms('After-off wait'),
  UpHeight: n('Retract height', 'mm', 0, 1000),
  NoFollow: flag('Follow sheet surface'),
  ShortDistNoUp: flag('Keep height on short moves'),
  ShortDistGasKeepOn: flag('Keep gas on short moves'),
  NoCloseGasInManu: flag('Keep gas during processing'),
  NoManu: flag('Layer skipped'),
  UD_UpEnable: flag('Cut start'), UD_UpAdvEnable: flag('Override duty and frequency'), UD_UpLen: n('Region length', 'mm'), UD_UpSpeed: n('Region speed', 'mm/s', 0.01), UD_UpDuty: percent('Region duty'), UD_UpFreq: hertz('Region frequency'),
  UD_DownEnable: flag('Cut end'), UD_DownAdvEnable: flag('Override duty and frequency'), UD_DownLen: n('Region length', 'mm'), UD_DownSpeed: n('Region speed', 'mm/s', 0.01), UD_DownDuty: percent('Region duty'), UD_DownFreq: hertz('Region frequency'),
  SlowStart: flag('Slow start (legacy)'), SlowStartLength: n('Slow start length', 'mm'), SlowStartSpeed: n('Slow start speed', 'mm/s', 0.01),
  DrillHeight: n('Nozzle height', 'mm', 0, 1000, 'Height above the sheet for this piercing stage.'),
  DrillDelay: ms('Duration', 'One stage duration: the stationary dwell, or the transition time with progressive descent.'),
  GradualTime: ms('Duration'), FocusGradualTime: ms('Duration'),
  DrillPower: percent('Duty cycle'), DrillGasPressure: n('Gas pressure', 'bar', 0, 100), DrillFreq: hertz('Frequency'), DrillPeakCurrent: percent('Peak output'), DrillGasType: gas('Stage gas'),
  EnableGradualDrill: flag('Progressive descent'),
  BoltDrill_Enable: flag('Ramp duty / frequency'), BoltDrill_Power: percent('End duty'), BoltDrill_Freq: hertz('End frequency'),
  BeforeLaserOffDelay: ms('Laser-on dwell'), AfterLaserOffDelay: ms('Gas afterflow'),
  EnableSmoothPierce: flag('Smooth piercing'),
  SmoothPierceDrillHeight: n('Nozzle height', 'mm', 0, 1000), SmoothPierceDrillPower: percent('Duty cycle'), SmoothPierceDrillFreq: hertz('Frequency'), SmoothPierceDrillPeakCurrent: percent('Peak output'),
  SmoothPierceDrillTime_ms: ms('Smooth duration (stored)'),
  PreDrill: flag('Batch pre-piercing'), AfterPreDrillMustDrillBeforeCut: flag('Pierce again before cutting'), PreDrillIsNotUp: flag('Keep head down within a batch'),
  WithFilm: flag('Film removal'),
  EnableContourShift: flag('Contour shift'), ContourShiftXDist: n('Shift X', 'mm', -100000), ContourShiftYDist: n('Shift Y', 'mm', -100000),
  CleanResidue_Enable: flag('Slag removal'), CleanResidue_WorkH: n('Nozzle height', 'mm', 0, 1000), CleanResidue_WorkV: n('Movement speed', 'mm/s', 0.01), CleanResidue_GasType: gas('Gas'), CleanResidue_GasP: n('Gas pressure', 'bar', 0, 100),
  CleanResidue_PeakCurrent: percent('Peak output'), CleanResidue_Power: percent('Duty cycle'), CleanResidue_Freq: hertz('Frequency'), CleanResidue_WorkR: n('Spiral radius', 'mm'), CleanResidue_SpiralTimes: whole('Spiral turns', '', 1, 4096),
  PowerAdjustWithSpeed: flag('Adjust duty with speed'), FreqAdjustWithSpeed: flag('Adjust frequency with speed'), PWMCurveNodes: text('Duty curve'), FreqCurveNodes: text('Frequency curve'), PowerCurveSmoothType: whole('Duty graph style', '', 0, 10), FreqCurveSmoothType: whole('Frequency graph style', '', 0, 10),
  ZFVibAbatType: whole('Suppression type', '', 0, 3), ZFVibAbat_Level: whole('Thin plate', '', 0, 255), ZFVibAbat_Level_Thick: whole('Thick plate', '', 0, 255),
  // Kept as the file holds them: optical focus this machine sets by hand, and switches nothing reads.
  CutFocusPos: n('Optical focus', 'mm', -1000, 1000), DrillFocusPos: n('Optical focus', 'mm', -1000, 1000), EnableFocusGradual: flag('Progressive focus'), FocusGradualEndPos: n('Focus end position', 'mm', -1000, 1000),
  SmoothPierceDrillFocusPos: n('Optical focus', 'mm', -1000, 1000), CleanResidue_WorkFocus: n('Optical focus', 'mm', -1000, 1000),
  PreLaserOnFactor: n('Pre laser-on factor', ''), ZFVibAbat_Coef: n('Suppression coefficient', ''), LeadLineParam_Enable: flag('Lead process (legacy)'), DrillTime: ms('Piercing time (unused)'),
  LayerFileName: text('Layer name'), ManuType: whole('Machining type', '', 0, 7), Note: text('Note'),
};

const STAGE_KEY = /^(DrillHeight|DrillPower|DrillFreq|DrillPeakCurrent|DrillGasType|DrillGasPressure|DrillDelay|DrillFocusPos|DrillTime|EnableGradualDrill|GradualTime|EnableFocusGradual|FocusGradualEndPos|FocusGradualTime|BeforeLaserOffDelay|AfterLaserOffDelay)(\d+)$|^BoltDrill_(Enable|Power|Freq)_(\d+)$/;
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
  if (f.kind === 'gas') return GAS[Number(value)] ?? `Selection ${value}`;
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
  return m === 'smooth' ? 'Smooth piercing' : m === 'staged' ? `${stageCount(v)} piercing stage${stageCount(v) === 1 ? '' : 's'}` : 'No piercing';
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
  /** Whether the laser takes a peak output level. */
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
