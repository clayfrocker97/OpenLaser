import { describe, expect, it } from 'vitest';
import { current, needed, ordered, suggestStock, suits, usable } from './stock-plan';
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

  it('estimates the area the parts need, the selected part repeated', () => {
    const box = { closed: true, depth: 0, paths: [{ points: square(0, 10) }] };
    const hole = { closed: true, depth: 1, paths: [{ points: square(2, 2) }] };
    const draft = { groups: [[0, 1]], preview: { contours: [box, hole] } } as never;
    expect(needed(draft, 3)).toBeCloseTo(300 * 1.3);
  });

  it('uses the smallest remnant that holds the job, then the rack, then new sheets', () => {
    const plan = suggestStock({
      need: 5000, remnants: [remnant('big', 300), remnant('fits', 100), remnant('small', 50)],
      rack: [rack('s', 500, 2), rack('l', 1000, 1), rack('none', 800, 0)], fallback: [1000, 1000],
    });
    expect(plan).toEqual([
      { kind: 'remnant', id: 'fits' },
      { kind: 'stock', id: 'l', count: 1 },
      { kind: 'stock', id: 's', count: 2 },
      { kind: 'sheet', width: 1000, height: 1000, count: null },
    ]);
  });

  it('starts a large job on the largest remnant only when it takes a fair share', () => {
    const remnants = [remnant('big', 100), remnant('small', 20)];
    expect(suggestStock({ need: 30_000, remnants, rack: [], fallback: [1, 1] })[0]).toEqual({ kind: 'remnant', id: 'big' });
    expect(suggestStock({ need: 100_000, remnants, rack: [], fallback: [1, 1] })[0]!.kind).toBe('sheet');
  });

  it('keeps a remnant the operator chose', () => {
    const plan = suggestStock({ need: 1, remnants: [remnant('a', 10), remnant('b', 90)], rack: [], fallback: [1, 1], chosen: 'b' });
    expect(plan[0]).toEqual({ kind: 'remnant', id: 'b' });
  });

  it('checks a plan against what is on hand now', () => {
    const plan = current([
      { kind: 'stock', id: 's', count: 2 }, { kind: 'stock', id: 's', count: 2 }, { kind: 'stock', id: 'gone', count: 1 },
      { kind: 'remnant', id: 'r' }, { kind: 'remnant', id: 'r' }, { kind: 'remnant', id: 'used' },
      { kind: 'sheet', width: 5, height: 5, count: null }, { kind: 'sheet', width: 5, height: 5, count: null },
    ], [rack('s', 500, 3)], [remnant('r', 10)]);
    expect(plan).toEqual([
      { kind: 'remnant', id: 'r' }, { kind: 'stock', id: 's', count: 3 }, { kind: 'sheet', width: 5, height: 5, count: null },
    ]);
  });

  it('orders remnants, then the rack, then new sheets', () => {
    const plan = ordered([
      { kind: 'sheet', width: 1, height: 1, count: null }, { kind: 'stock', id: 's', count: 1 }, { kind: 'remnant', id: 'r' },
    ]);
    expect(plan.map((s) => s.kind)).toEqual(['remnant', 'stock', 'sheet']);
  });
});
