import { describe, expect, it } from 'vitest';
import { current, oneMore, suits, usable } from './stock-plan';
import type { SheetView, StockItem } from '../api';

const square = (x: number, size: number): Array<[number, number]> => [[x, x], [x + size, x], [x + size, x + size], [x, x + size]];
const remnant = (id: string, size: number, cut = 0): SheetView => ({
  id, revision: 0, state: 'remnant', used: false, name: id, job: null, mode: 'fiber', material: 'Steel', thickness_mm: 2,
  at: 0, reported: false, boundary_known: true, bounds: { min: { x: 0, y: 0 }, max: { x: size, y: size } },
  outline: square(0, size), cutouts: cut ? [square(0, cut)] : [], clearance: 0, folder: null,
});
const rack = (id: string, width: number, quantity: number): StockItem => ({
  id, material: 'Steel', thickness_mm: 2, laser: 'fiber', width_mm: width, height_mm: width, quantity, folder: null,
});

describe('stock plan', () => {
  it('matches stock to the recipe like the server', () => {
    const recipe = { laser: 'fiber' as const, name: ' steel ', thickness_mm: 2 };
    expect(suits({ laser: 'fiber', material: 'Steel', thickness_mm: 2 }, recipe)).toBe(true);
    expect(suits({ laser: 'fiber', material: 'Steel', thickness_mm: 3 }, recipe)).toBe(false);
    expect(suits({ laser: 'co2', material: 'Steel', thickness_mm: 2 }, recipe)).toBe(false);
    expect(suits({ laser: 'fiber', material: 'Steel', thickness_mm: 2 }, null)).toBe(true);
  });

  it('counts a remnant by what is left on it', () => {
    expect(usable(remnant('a', 100, 50))).toBe(7500);
  });

  it('checks a plan against what is on hand now', () => {
    const plan = current([
      { kind: 'stock', id: 's', count: 2 }, { kind: 'stock', id: 's', count: 2 }, { kind: 'stock', id: 'gone', count: 1 },
      { kind: 'remnant', id: 'r' }, { kind: 'remnant', id: 'r' }, { kind: 'remnant', id: 'used' },
      { kind: 'sheet', width: 5, height: 5, count: 1 }, { kind: 'sheet', width: 5, height: 5, count: 1 },
    ], [rack('s', 500, 3)], [remnant('r', 10)]);
    expect(plan).toEqual([
      { kind: 'stock', id: 's', count: 3 }, { kind: 'remnant', id: 'r' }, { kind: 'sheet', width: 5, height: 5, count: 2 },
    ]);
  });

  it('adds one more of a listed sheet while there is one', () => {
    expect(oneMore({ kind: 'sheet', width: 5, height: 5, count: 1 }, [])).toEqual({ kind: 'sheet', width: 5, height: 5, count: 2 });
    expect(oneMore({ kind: 'stock', id: 's', count: 1 }, [rack('s', 500, 2)])).toEqual({ kind: 'stock', id: 's', count: 2 });
    expect(oneMore({ kind: 'stock', id: 's', count: 2 }, [rack('s', 500, 2)])).toBeNull();
    expect(oneMore({ kind: 'remnant', id: 'r' }, [])).toBeNull();
});

});
