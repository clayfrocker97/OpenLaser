import { afterEach, describe, expect, it, vi } from 'vitest';
import { diagnosticText, displayNumber, inputValue, quantity, sourceInput, toDisplay, toSource, unitLabel, units } from './units.svelte';
import { osk } from './osk.svelte';
import { bare, field, shown } from './recipe';
import { fieldUnit, xmlValue } from './machine-settings';
import { conflictText } from './review-values';

afterEach(() => { units.set('metric'); osk.close(); vi.unstubAllGlobals(); });

describe('imperial presentation with metric storage', () => {
  it('converts lengths, feed rates, area, pressure and temperature using their physical dimensions', () => {
    units.set('imperial');
    expect(quantity(25.4, 'mm')).toBe('1.0000 in');
    expect(quantity(25.4, 'mm/s')).toBe('60.000 in/min');
    expect(toSource(60, 'mm/s')).toBeCloseTo(25.4, 12);
    expect(toDisplay(645.16, 'mm²')).toBeCloseTo(1, 12);
    expect(toSource(1, 'in')).toBe(1);
    expect(toDisplay(1, 'bar')).toBeCloseTo(14.503773773, 8);
    expect(toSource(14.503773773, 'bar')).toBeCloseTo(1, 8);
    expect(toDisplay(20, '°C')).toBe(68);
    expect(toSource(68, '°C')).toBe(20);
    expect(unitLabel('mm/s²')).toBe('in/s²');
    expect(toDisplay(100, 'counts/mm')).toBe(2540);
  });

  it('preserves the original metric digits when an unchanged converted value is accepted', () => {
    units.set('imperial');
    for (const value of [31.039999999999999, 1 / 3, 0.000000001, -9876.123456789]) {
      expect(sourceInput(inputValue(value, 'mm'), value, 'mm')).toBe(value);
    }
    expect(sourceInput('1', 3, 'mm')).toBeCloseTo(25.4, 12);
    expect(sourceInput('', 3, 'mm')).toBeNaN();
    expect(Number(displayNumber(0.001, 'mm'))).toBeGreaterThan(0);
  });

  it('keeps keypad conversions fixed to the units in which the entry was opened', () => {
    vi.stubGlobal('HTMLTextAreaElement', class {});
    units.set('imperial');
    let stored = 1;
    osk.number('Fixture X', stored, 'mm', value => { stored = value; });
    expect(osk.unit).toBe('in');
    expect(stored).toBe(1);
    units.set('metric');
    osk.key('1'); osk.key('done');
    expect(stored).toBeCloseTo(25.4, 12);
  });

  it('changes a persistent display preference without changing recipe or machine data', () => {
    const setItem = vi.fn(); vi.stubGlobal('localStorage', { setItem });
    const recipe = { CutSpeed: '25.4', CutHeight: '25.4', CutAirPressure: '1', CutFreq: '5000', FocusGradualTime: '500' };
    const before = JSON.stringify(recipe);
    units.set('imperial');
    expect(setItem).toHaveBeenCalledWith('ol-units', 'imperial');
    expect(shown('CutSpeed', recipe.CutSpeed)).toBe('60 in/min');
    expect(bare('CutHeight', recipe.CutHeight)).toBe('1');
    expect(shown('CutAirPressure', recipe.CutAirPressure)).toBe('14.5 psi');
    expect(shown('CutFreq', recipe.CutFreq)).toBe('5000 Hz');
    expect(field('FocusGradualTime').unit).toBe('ms');
    expect(shown('FocusGradualTime', recipe.FocusGradualTime)).toBe('500 ms');
    expect(JSON.stringify(recipe)).toBe(before);
    units.set('metric');
    expect(shown('CutSpeed', recipe.CutSpeed)).toBe('25.4 mm/s');
  });

  it('converts explicit XML dimensions while preserving rotary axes, ratios and controller words', () => {
    units.set('imperial');
    const axis = { path: '/ParameterRoot/PMachineAxisConfig_0/MAC', name: 'SpeedRatio', value: '25.4' };
    expect(xmlValue(axis, axis.value)).toBe('1.00000 in/rev');
    expect(fieldUnit({ path: '/ParameterRoot/PManuParam/MC', name: 'JogFastSpeed' })).toBe('mm/s');
    expect(fieldUnit({ path: '/ParameterRoot/PManuParam/MC', name: 'SplineAccuracyRate' })).toBe('');
    expect(fieldUnit({ path: '/ParameterRoot/PLayerParam1/GP', name: 'DrillHeight2' })).toBe('mm');
    const rotary = [{ ...axis, name: 'IsRotatingShaft', value: '1' }];
    expect(xmlValue(axis, '25.4', rotary)).toBe('25.40 °/rev');
    expect(xmlValue({ ...axis, name: 'WritePluse' }, '8000')).toBe('8000');
    expect(axis.value).toBe('25.4');
  });

  it('formats pending position and recipe conflicts without converting transform scale or angles', () => {
    units.set('imperial');
    expect(conflictText('[25.4,50.8]', '/placement/origin')).toBe('["1.0000 in","2.0000 in"]');
    expect(conflictText('"25.4"', '/recipe/attributes/CutSpeed')).toBe('60 in/min');
    expect(conflictText('[1,0,0,1,25.4,50.8]', '/placed/0/transform')).toBe('[1,0,0,1,"1.0000 in","2.0000 in"]');
    expect(conflictText('90', '/features/leads/entry/angle')).toBe('90');
  });

  it('uses imperial quantities in dimensional machine refusals without rewriting source names', () => {
    units.set('imperial');
    expect(diagnosticText('travel at 25.4000 mm')).toBe('travel at 1 in');
    expect(diagnosticText('pressure 0–100 bar')).toBe('pressure 0 – 1450.38 psi');
    expect(diagnosticText('Basswood-3mm.xml at port 100')).toBe('Basswood-3mm.xml at port 100');
  });
});
