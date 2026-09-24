import { describe, expect, it } from 'vitest';
import { calibrationStatus, duration, gasLabel, gasOfSelector, litres, measuredLitres, money, pricePerM3, totals, volume } from './gas';
import type { Consumption, GasCosts, RunRecord } from '../api';
import { toDisplay, toSource, unitLabel } from './units.svelte';

const consumption = (litresUsed: number | null, cost: number | null, laser = 60): Consumption => ({
  laser, pierces: 2, cut: 100, nozzle: null, cost,
  gases: [{ gas: 'nitrogen', pressure: 12, seconds: 70, flow: litresUsed === null ? null : litresUsed / 70 * 60, litres: litresUsed, cost }],
});
const run = (c: Consumption, outcome: 'done' | 'stopped' = 'done'): RunRecord => ({ id: 'r', key: 'job-1', job: '1', name: 'Plate', started: 1, finished: 2, outcome, fraction: outcome === 'done' ? 1 : 0.5, consumption: c, currency: '€' });

describe('gas and laser figures', () => {
  it('formats money, times and volumes as the card shows them', () => {
    expect(money(3.456, '$')).toBe('$3.46');
    expect(money(null, '$')).toBe('—');
    expect(duration(45)).toBe('45 s');
    expect(duration(72)).toBe('1.2 min');
    expect(duration(4680)).toBe('1.3 h');
    expect(volume(850.4)).toBe('850 L');
    expect(volume(2100)).toBe('2.10 m³');
    expect(volume(null)).toBe('—');
  });
  it('adds up the run history and keeps unknown volumes and prices unknown', () => {
    const t = totals([run(consumption(500, 2)), run(consumption(250, 1, 30), 'stopped')]);
    expect(t).toEqual({ runs: 2, laser: 90, litres: 750, cost: 3, currency: '€' });
    const unknown = totals([run(consumption(500, 2)), run(consumption(null, null))]);
    expect(unknown.litres).toBeNull();
    expect(unknown.cost).toBeNull();
    expect(totals([], '$')).toEqual({ runs: 0, laser: 0, litres: 0, cost: 0, currency: '$' });
    expect(litres(consumption(12, 1))).toBe(12);
    expect(gasLabel(consumption(12, 1))).toBe('N₂');
  });
  it('turns flow-test readings into litres', () => {
    // A 50 L cylinder dropping 2 bar holds about 99 standard litres less.
    expect(measuredLitres('pressure', 'nitrogen', { before: 200, after: 198, capacity: 50, seconds: 60 })).toBeCloseTo(98.69, 2);
    expect(measuredLitres('weight', 'oxygen', { before: 60.2, after: 60, capacity: 0, seconds: 60 })).toBeCloseTo(150.2, 1);
    expect(measuredLitres('meter', 'air', { before: 120, after: 0, capacity: 0, seconds: 60 })).toBe(120);
    expect(measuredLitres('litres', 'air', { before: 0, after: 0, capacity: 0, seconds: 60 })).toBeNull();
    expect(measuredLitres('pressure', 'air', { before: 100, after: 110, capacity: 50, seconds: 60 })).toBeNull();
  });
  it('derives prices, routes and calibration status', () => {
    expect(pricePerM3({ kind: 'refill', price: 90, volume: 9 })).toBe(10);
    expect(pricePerM3({ kind: 'refill', price: 0, volume: 9 })).toBeNull();
    expect(pricePerM3({ kind: 'compressor', cost_per_hour: 2 })).toBeNull();
    expect([0, 1, 2, 3, 4, 5].map(gasOfSelector)).toEqual(['air', 'oxygen', 'nitrogen', 'air', 'oxygen', 'nitrogen']);
    const supply = { source: { kind: 'bulk', price_per_m3: 3 }, flow: { kind: 'estimated' }, calibration: null } as const;
    const costs: GasCosts = { currency: '$', nitrogen: supply, oxygen: { ...supply, flow: { kind: 'manual', rate: 40 } }, air: { ...supply, calibration: { factor: 1.12, at: 0, pressure: 6, nozzle: { diameter: 1.5, kind: 'single' }, estimated: 100, measured: 112 } } };
    expect(calibrationStatus(costs, 'nitrogen')).toMatch(/Not calibrated/);
    expect(calibrationStatus(costs, 'oxygen')).toMatch(/Manual flow/);
    expect(calibrationStatus(costs, 'air')).toMatch(/^Calibrated ×1\.12 · .* · 6\.00 bar · 1\.50 mm single$/);
  });
});

describe('imperial gas units', () => {
  it('shows gas volumes in cubic feet', () => {
    expect(volume(850, 'imperial')).toBe('30.0 ft³');
    expect(volume(28316.846592, 'imperial')).toBe('1,000 ft³');
    expect(volume(850, 'metric')).toBe('850 L');
  });

  it('converts flow, cylinder volume, weight and price per volume', () => {
    expect(toDisplay(10, 'L/min', 'imperial')).toBeCloseTo(21.1888, 3);
    expect(unitLabel('L/min', 'imperial')).toBe('ft³/h');
    expect(toDisplay(9, 'm³', 'imperial')).toBeCloseTo(317.832, 2);
    expect(toDisplay(10, 'kg', 'imperial')).toBeCloseTo(22.0462, 3);
    // $52 per m³ is about $1.47 per cubic foot, and back again exactly.
    expect(toDisplay(52, '/m³', 'imperial')).toBeCloseTo(1.4725, 3);
    expect(toSource(toDisplay(52, '/m³', 'imperial'), '/m³', 'imperial')).toBeCloseTo(52, 9);
  });
});
