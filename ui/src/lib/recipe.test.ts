import { describe, expect, it } from 'vitest';
import { available, status } from './recipe';
import type { CapabilitiesView } from '../api';

describe('CO2 air assist', () => {
  it('offers only High Air, with pressure set at the regulator', () => {
    const a = available(undefined, 'co2');
    expect(a.gas).toBe(true);
    expect(a.gases).toEqual([3]);
    expect(a.fixedGas).toBe(3);
    expect(a.pressure(3)).toBe(false);
    expect(status('CutGasType', { CutGasType: '5' }, a)).toBe('inactive');
    expect(status('DrillGasType0', { DrillGasType0: '1' }, a)).toBe('inactive');
  });

  it('does not substitute another wired gas when High Air is missing', () => {
    const caps = [{ laser: 'co2', gases: [{ selector: 5, valve: true, pressure: false }] }] as CapabilitiesView[];
    const a = available(caps, 'co2');
    expect(a.gas).toBe(false);
    expect(a.gases).toEqual([]);
    expect(a.fixedGas).toBe(3);
  });
});
