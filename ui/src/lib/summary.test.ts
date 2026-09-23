import { afterEach, describe, expect, it } from 'vitest';
import { SUMMARY_LABELS, nozzleText, pierceText, summaryItems, summaryLine, summaryOf } from './summary';
import { field } from './recipe';
import { units } from './units.svelte';

afterEach(() => units.set('metric'));

const values = { CutSpeed: '25', CutPeakCurrent: '90', CutPower: '80', CutFreq: '5000', CutGasType: '5', CutAirPressure: '12', CutHeight: '0.8', OpenLaserNozzleDiameter: '1.5', OpenLaserNozzleType: 'double', OpenLaserManualFocus: '-2', OpenLaserLens: '150', ManuType: '3' };

describe('the material summary', () => {
  it('lists every value in one order under one set of names', () => {
    const items = summaryItems(summaryOf(values, 'fiber'));
    expect(items.map((i) => [i.label, i.value])).toEqual([
      ['Speed', '25 mm/s'], ['Power', '90 %'], ['Duty', '80 %'], ['Frequency', '5000 Hz'], ['Gas', 'High N₂ · 12 bar'],
      ['Nozzle', '1.5 mm double'], ['Focus', '-2 mm'], ['Lens', '150 mm'], ['Cut height', '0.8 mm'], ['Pierce', '2 stages'],
    ]);
  });

  it('never calls the duty cycle power', () => {
    expect(SUMMARY_LABELS.power).toBe('Power');
    expect(SUMMARY_LABELS.duty).toBe('Duty');
    expect(field('CutPeakCurrent').label).toBe('Power');
    expect(field('CutPower').label).toBe('Duty');
    expect(field('CutDuty').label).toBe('Duty');
    expect(field('DrillPower2').label).toBe('Duty');
    expect(field('DrillPeakCurrent2').label).toBe('Power');
    expect(field('CutHeight').label).toBe('Cut height');
  });

  it('marks what the recipe does not say and leaves it off the line', () => {
    const source = summaryOf({ CutSpeed: '10', CutDuty: '40', EnableSmoothPierce: '1' }, 'co2');
    const items = summaryItems(source);
    expect(items.find((i) => i.key === 'power')).toMatchObject({ value: '—', set: false });
    expect(items.find((i) => i.key === 'gas')).toMatchObject({ value: 'High Air', set: true });
    expect(summaryLine(source)).toBe('Speed 10 mm/s · Duty 40 % · Gas High Air · Pierce Smooth');
  });

  it('shows lengths and pressures in the chosen units', () => {
    units.set('imperial');
    const line = summaryLine(summaryOf(values, 'fiber'), ['speed', 'gas', 'height']);
    expect(line).toBe('Speed 59.055 in/min · Gas High N₂ · 174.05 psi · Cut height 0.0315 in');
  });

  it('reads nozzle and piercing as one value each', () => {
    expect(nozzleText('2.0', 'single')).toBe('2 mm single');
    expect(nozzleText(null, 'double')).toBe('Double');
    expect(nozzleText(null, null)).toBeNull();
    expect(pierceText({ pierce_stages: 0, smooth_pierce: false })).toBe('None');
    expect(pierceText({ pierce_stages: 1, smooth_pierce: false })).toBe('1 stage');
  });
});
