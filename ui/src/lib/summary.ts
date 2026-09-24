// The one material summary every screen shows: the library, the recipe,
// Setup's material card and picker, Run and the phone. The same values in
// the same order under the same names, so an operator reads a recipe the
// same way wherever it appears.
//
// Wording (ui/DESIGN.md, "Power and duty"): M-Laser's names. Peak Current
// is the laser's output while on (`CutPeakCurrent`); Cut Power is the PWM
// duty (`CutPower` on fiber, `CutDuty` on CO₂).

import type { LaserMode, RecipeSummary } from '../api';
import { GAS, isOn, stageCount, shown, type Values } from './recipe';
import { plural } from './format';

/** What a summary is made from: a recipe, a machine bank, a running program's material or an import preview. */
export interface SummarySource {
  laser: LaserMode;
  /** The gas as the library names it. */
  gas: string;
  summary: RecipeSummary;
}

export type SummaryKey = 'speed' | 'power' | 'duty' | 'frequency' | 'gas' | 'nozzle' | 'focus' | 'lens' | 'height' | 'pierce';

export interface SummaryItem {
  key: SummaryKey;
  label: string;
  /** The value with its unit, or "—" when the recipe does not say. */
  value: string;
  /** Whether the recipe holds a value. */
  set: boolean;
}

/** The labels, in the order every summary uses. */
export const SUMMARY_LABELS: Record<SummaryKey, string> = {
  speed: 'Cut Speed',
  power: 'Peak Current',
  duty: 'Cut Power',
  frequency: 'Cut Freq',
  gas: 'Gas',
  nozzle: 'Nozzle',
  focus: 'Focus',
  lens: 'Lens',
  height: 'Cut Height',
  pierce: 'Pierce',
};

const ORDER = Object.keys(SUMMARY_LABELS) as SummaryKey[];
const present = (v: string | null | undefined): v is string => v !== null && v !== undefined && v.trim() !== '';

/** The nozzle as one value: bore and construction. */
export function nozzleText(diameter: string | null | undefined, kind: string | null | undefined): string | null {
  const type = kind === 'double' ? 'double' : kind === 'single' ? 'single' : '';
  if (!present(diameter)) return type ? type[0]!.toUpperCase() + type.slice(1) : null;
  return `${shown('OpenLaserNozzleDiameter', diameter)}${type ? ` ${type}` : ''}`;
}

/** The piercing as one value. */
export function pierceText(summary: Pick<RecipeSummary, 'pierce_stages' | 'smooth_pierce'>): string {
  if (summary.smooth_pierce) return 'Smooth';
  const n = summary.pierce_stages;
  return n ? plural(n, 'stage') : 'None';
}

/** Every summary value in order; `—` where the recipe does not say. */
export function summaryItems(source: SummarySource): SummaryItem[] {
  const s = source.summary;
  const pressure = present(s.pressure) ? shown('CutAirPressure', s.pressure) : null;
  const values: Record<SummaryKey, string | null> = {
    speed: present(s.speed) ? shown('CutSpeed', s.speed) : null,
    power: present(s.peak) ? shown('CutPeakCurrent', s.peak) : null,
    duty: present(s.duty) ? shown('CutPower', s.duty) : null,
    frequency: present(s.frequency) ? shown('CutFreq', s.frequency) : null,
    gas: present(source.gas) ? (pressure ? `${source.gas} · ${pressure}` : source.gas) : pressure,
    nozzle: nozzleText(s.setup.nozzle_diameter_mm, s.setup.nozzle),
    focus: present(s.setup.focus_mm) ? shown('OpenLaserManualFocus', s.setup.focus_mm) : null,
    lens: present(s.setup.lens_mm) ? shown('OpenLaserLens', s.setup.lens_mm) : null,
    height: present(s.height) ? shown('CutHeight', s.height) : null,
    pierce: pierceText(s),
  };
  return ORDER.map((key) => ({ key, label: SUMMARY_LABELS[key], value: values[key] ?? '—', set: values[key] !== null }));
}

/** The summary on one line, values that are set only: `Speed 25 mm/s · Power 90 % · …`. */
export function summaryLine(source: SummarySource, keys: readonly SummaryKey[] = ORDER): string {
  return summaryItems(source).filter((i) => i.set && keys.includes(i.key)).map((i) => `${i.label} ${i.value}`).join(' · ');
}

/** The summary of a recipe's values as they stand in the editor, staged
 * edits included; the server's `RecipeSummary` reads the same keys. */
export function summaryOf(values: Values, laser: LaserMode): SummarySource {
  const get = (key: string): string | null => values[key] ?? null;
  const [duty, other] = laser === 'co2' ? ['CutDuty', 'CutPower'] : ['CutPower', 'CutDuty'];
  const gas = laser === 'co2' ? '3' : get('CutGasType');
  const kind = get('OpenLaserNozzleType');
  return {
    laser,
    gas: gas === null ? '' : GAS[Number(gas)] ?? `Selection ${gas}`,
    summary: {
      speed: get('CutSpeed'),
      peak: get('CutPeakCurrent'),
      duty: get(duty!) ?? get(other!),
      frequency: get('CutFreq'),
      gas,
      pressure: laser === 'co2' ? null : get('CutAirPressure'),
      height: get('CutHeight'),
      setup: {
        nozzle_diameter_mm: get('OpenLaserNozzleDiameter'),
        nozzle: kind === 'single' || kind === 'double' ? kind : null,
        focus_mm: get('OpenLaserManualFocus'),
        lens_mm: get('OpenLaserLens'),
      },
      pierce_stages: stageCount(values),
      smooth_pierce: isOn(values['EnableSmoothPierce']),
    },
  };
}
