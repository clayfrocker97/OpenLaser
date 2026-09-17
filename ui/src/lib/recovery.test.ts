import { describe, expect, it } from 'vitest';
import { restartAt } from './recovery';
import type { Compiled, RecoveryStep } from '../api';

const step = (kind: 'cut' | 'film', status = 'pending'): RecoveryStep => ({ pass: { ordinal: 0, kind, instances: [], omitted_cooling: 0 }, status, length_mm: 20, executed: [] });
const program = (moves: Compiled['moves']): Compiled => ({ dry_run: false, seconds: 1, plan: [], pierces: [], blocks: 1, moves });

describe('touch restart selection', () => {
  it('finds the fraction along actual runs without bridging a travel gap', () => {
    const compiled = program([{ pass: 0, kind: 'cut', points: [[0, 0], [10, 0]] }, { pass: 0, kind: 'travel', points: [[10, 0], [100, 0]] }, { pass: 0, kind: 'cut', points: [[100, 0], [110, 0]] }]);
    expect(restartAt(compiled, [step('cut')], [105, 1], 2)).toEqual({ pass: 0, fraction: .75 });
    expect(restartAt(compiled, [step('cut')], [50, 0], 2)).toBeNull();
  });
  it('chooses cutting over coincident film and excludes skipped paths', () => {
    const compiled = program([{ pass: 0, kind: 'film', points: [[0, 0], [10, 0]] }, { pass: 1, kind: 'cut', points: [[0, 0], [10, 0]] }]);
    expect(restartAt(compiled, [step('film'), step('cut')], [5, 0], 2)?.pass).toBe(1);
    expect(restartAt(compiled, [step('film'), step('cut', 'skipped')], [5, 0], 2)?.pass).toBe(0);
  });
});
