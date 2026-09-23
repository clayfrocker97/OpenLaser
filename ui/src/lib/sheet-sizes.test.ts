import { describe, expect, it } from 'vitest';
import { fits, oriented } from './sheet-sizes';

describe('sheet sizes', () => {
  it('lays the long side along the long side of the bed', () => {
    expect(oriented(3000, 1500, { width: 3050, height: 1550 })).toEqual([3000, 1500]);
    expect(oriented(3000, 1500, { width: 1550, height: 3050 })).toEqual([1500, 3000]);
    expect(oriented(3000, 1500, null)).toEqual([3000, 1500]);
  });

  it('checks the sheet against the bed', () => {
    expect(fits([3000, 1500], { width: 3050, height: 1550 })).toBe(true);
    expect(fits([4000, 2000], { width: 3050, height: 1550 })).toBe(false);
    expect(fits([4000, 2000], null)).toBe(true);
  });
});
