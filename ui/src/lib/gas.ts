// Gas and laser cost presentation. The server prices jobs and runs
// (crates/openlaser-server/src/gas); this file only formats the figures,
// totals the run history and turns a flow-test reading into litres.
import type { Consumption, GasCosts, GasKind, GasRouteView, RunRecord, Source } from '../api';
import { quantity, toDisplay, units } from './units.svelte';

export const GAS_NAMES: Record<GasKind, string> = { nitrogen: 'N₂', oxygen: 'O₂', air: 'Air' };
export const GAS_COLORS: Record<GasKind, string> = { nitrogen: 'var(--gas-n2)', oxygen: 'var(--gas-o2)', air: 'var(--gas-air)' };
export const GASES: GasKind[] = ['nitrogen', 'oxygen', 'air'];

/** The gas a valve selector carries: low/high air, oxygen, nitrogen. */
export const gasOfSelector = (selector: number): GasKind => (['air', 'oxygen', 'nitrogen'] as const)[selector % 3]!;

/** The machine's valve routes that carry `gas`. */
export const routesFor = (routes: GasRouteView[], gas: GasKind): GasRouteView[] => routes.filter(r => r.valve && gasOfSelector(r.selector) === gas);

/** Money to two decimals after the shop's symbol. */
export const money = (value: number | null | undefined, currency: string): string =>
  value === null || value === undefined || !Number.isFinite(value) ? '—' : `${value < 0 ? '−' : ''}${currency}${Math.abs(value).toFixed(2)}`;

/** "45 s", "1.2 min" or "1.3 h". */
export function duration(seconds: number): string {
  const s = Math.max(0, seconds);
  if (Math.round(s) < 60) return `${Math.round(s)} s`;
  if (s < 3600) return `${(s / 60).toFixed(1)} min`;
  return `${(s / 3600).toFixed(1)} h`;
}

/** "850 L" or "2.10 m³"; in imperial "30.0 ft³" or "1,250 ft³". */
export function volume(litres: number | null | undefined, system = units.system): string {
  if (litres === null || litres === undefined || !Number.isFinite(litres)) return '—';
  const l = Math.max(0, litres);
  if (system === 'imperial') {
    const cubicFeet = toDisplay(l, 'L', system);
    return cubicFeet < 100 ? `${cubicFeet.toFixed(1)} ft³` : `${Math.round(cubicFeet).toLocaleString('en-US')} ft³`;
  }
  return Math.round(l) < 1000 ? `${Math.round(l)} L` : `${(l / 1000).toFixed(2)} m³`;
}

/** Litres of a consumption, or null while any gas's flow is unknown. */
export function litres(c: Pick<Consumption, 'gases'>): number | null {
  let total = 0;
  for (const line of c.gases) {
    if (line.litres === null) return null;
    total += line.litres;
  }
  return total;
}

/** What the gas label reads: the one gas used, or "Gas" for a mix. */
export function gasLabel(c: Pick<Consumption, 'gases'>): string {
  const kinds = [...new Set(c.gases.map(g => g.gas))];
  return kinds.length === 1 ? GAS_NAMES[kinds[0]!] : 'Gas';
}

/** One job's run history added up. Cost is null when any run is unpriced. */
export type Totals = { runs: number; laser: number; litres: number | null; cost: number | null; currency: string };
export function totals(runs: RunRecord[], fallbackCurrency = '$'): Totals {
  let laser = 0, volumeTotal: number | null = 0, cost: number | null = 0;
  for (const run of runs) {
    laser += run.consumption.laser;
    const l = litres(run.consumption);
    volumeTotal = volumeTotal === null || l === null ? null : volumeTotal + l;
    cost = cost === null || run.consumption.cost === null ? null : cost + run.consumption.cost;
  }
  return { runs: runs.length, laser, litres: volumeTotal, cost, currency: runs[0]?.currency ?? fallbackCurrency };
}

/** The price of a standard cubic metre, as the server derives it. */
export function pricePerM3(source: Source): number | null {
  if (source.kind === 'refill') return source.price > 0 && source.volume > 0 ? source.price / source.volume : null;
  if (source.kind === 'bulk') return source.price_per_m3 > 0 ? source.price_per_m3 : null;
  return null;
}

/** Whether a supply has what it needs to price gas. */
export function priced(costs: GasCosts, gas: GasKind): boolean {
  const source = costs[gas].source;
  return source.kind === 'compressor' ? source.cost_per_hour > 0 : pricePerM3(source) !== null;
}

/** Standard litres per kilogram at 20 °C, for weighing a cylinder. */
export const LITRES_PER_KG: Record<GasKind, number> = { nitrogen: 858, oxygen: 751, air: 831 };
/** Standard atmosphere in bar: a cylinder's pressure drop over this is its litres per litre of water capacity. */
const ATMOSPHERE_BAR = 1.01325;

/** How the operator measures a flow test. */
export type Method = 'pressure' | 'weight' | 'meter' | 'litres';
export const METHODS: { id: Method; label: string; hint: string }[] = [
  { id: 'pressure', label: 'Cylinder pressure', hint: 'Read the cylinder gauge before and after' },
  { id: 'weight', label: 'Cylinder weight', hint: 'Weigh the cylinder before and after' },
  { id: 'meter', label: 'Flow meter', hint: 'Read the flow meter while the gas flows' },
  { id: 'litres', label: 'Volume used', hint: 'Enter the gas the test used' },
];

/** The litres a flow test used from the operator's readings.
 *  Pressure: the drop in bar times the cylinder's water capacity in litres,
 *  over one atmosphere (ideal gas at room temperature). Weight: kilograms
 *  times the gas's litres per kilogram. Meter: L/min over the test time. */
export function measuredLitres(method: Method, gas: GasKind, readings: { before: number; after: number; capacity: number; seconds: number }): number | null {
  const { before, after, capacity, seconds } = readings;
  let litresUsed: number;
  switch (method) {
    case 'pressure': litresUsed = (before - after) * capacity / ATMOSPHERE_BAR; break;
    case 'weight': litresUsed = (before - after) * LITRES_PER_KG[gas]; break;
    case 'meter': litresUsed = before * seconds / 60; break;
    case 'litres': litresUsed = before; break;
  }
  return Number.isFinite(litresUsed) && litresUsed > 0 ? litresUsed : null;
}

/** "Calibrated ×1.12 · 3 Jan · 12 bar · 1.5 mm single" or why there is none. */
export function calibrationStatus(costs: GasCosts, gas: GasKind): string {
  const supply = costs[gas];
  if (supply.flow.kind === 'manual') return 'Manual flow · no calibration needed';
  const c = supply.calibration;
  if (!c) return 'Not calibrated · nozzle estimate';
  const date = new Date(c.at * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
  return `Calibrated ×${c.factor.toFixed(2)} · ${date} · ${quantity(c.pressure, 'bar')} · ${quantity(c.nozzle.diameter, 'mm')} ${c.nozzle.kind}`;
}
