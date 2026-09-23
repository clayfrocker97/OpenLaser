import { describe, expect, it } from 'vitest';
import { plural } from './format';

describe('plural', () => {
  it('uses the singular for exactly one and the plural otherwise', () => {
    expect(plural(1, 'path')).toBe('1 path');
    expect(plural(0, 'item')).toBe('0 items');
    expect(plural(2, 'part')).toBe('2 parts');
    expect(plural(3, 'cut area')).toBe('3 cut areas');
  });

  it('takes an irregular plural', () => {
    expect(plural(1, 'match', 'matches')).toBe('1 match');
    expect(plural(4, 'match', 'matches')).toBe('4 matches');
  });
});
