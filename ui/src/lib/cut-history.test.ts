import { describe, expect, it } from 'vitest';
import type { Move } from '../api';
import { cutHistory } from './cut-history';

const line = (pass: number, points: [number, number][], kind: Move['kind'] = 'cut'): Move => ({ pass, points, kind });

describe('retained cut history', () => {
  it('keeps completed and partial original paths and excludes positioning moves', () => {
    const moves = [line(0, [[0, 0], [10, 0]]), line(1, [[10, 0], [20, 0]], 'travel'), line(1, [[20, 0], [20, 20]])];
    expect(cutHistory(moves, [{ executed: [[0, 1]] }, { executed: [[0, .25]] }])).toEqual([
      [[0, 0], [10, 0]], [[20, 0], [20, 5]],
    ]);
  });

  it('preserves separate recorded intervals without drawing across uncut gaps', () => {
    expect(cutHistory([line(0, [[0, 0], [10, 0], [10, 10]])], [{ executed: [[0, .25], [.75, 1]] }])).toEqual([
      [[0, 0], [5, 0]], [[10, 5], [10, 10]],
    ]);
  });

  it('measures a partial pass across its lead and cutting sections', () => {
    expect(cutHistory([line(0, [[0, 0], [10, 0]], 'lead_in'), line(0, [[10, 0], [10, 10]])], [{ executed: [[.25, .75]] }])).toEqual([
      [[5, 0], [10, 0]], [[10, 0], [10, 5]],
    ]);
  });

  it('does not fabricate history for missing evidence or zero-length segments', () => {
    expect(cutHistory([line(0, [[0, 0], [0, 0]])], [{ executed: [[0, 1]] }])).toEqual([]);
    expect(cutHistory([line(0, [[0, 0], [10, 0]])], [])).toEqual([]);
  });
});
