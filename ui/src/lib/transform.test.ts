import { describe, expect, it } from 'vitest';
import { about, after, apply, centre, mirror, mirrorVertical, rotation, scaling, svgMatrix, tenth, translate, type Point } from './transform';
import type { Transform } from '../api';

const close = (p: Point, q: Point) => { expect(p[0]).toBeCloseTo(q[0], 9); expect(p[1]).toBeCloseTo(q[1], 9); };

describe('similarity transforms', () => {
  it('applies a move', () => {
    expect(apply(translate(3, -2), [1, 1])).toEqual([4, -1]);
  });

  it('composes right to left', () => {
    const turn = rotation(90, [0, 0]);
    const move = translate(10, 0);
    // Move first, then turn: (1, 0) → (11, 0) → (0, 11).
    close(apply(after(turn, move), [1, 0]), [0, 11]);
    // Turn first, then move: (1, 0) → (0, 1) → (10, 1).
    close(apply(after(move, turn), [1, 0]), [10, 1]);
  });

  it('keeps the identity neutral', () => {
    const m: Transform = [2, 0.5, -0.5, 2, 7, 9];
    const identity: Transform = [1, 0, 0, 1, 0, 0];
    expect(after(identity, m)).toEqual(m);
    expect(after(m, identity)).toEqual(m);
  });

  it('turns counterclockwise about a centre, which stays put', () => {
    const c: Point = [5, 5];
    close(apply(rotation(90, c), c), c);
    close(apply(rotation(90, c), [10, 5]), [5, 10]);
    close(apply(rotation(180, c), [10, 5]), [0, 5]);
  });

  it('mirrors across the lines through a centre', () => {
    expect(apply(mirror([5, 0]), [7, 3])).toEqual([3, 3]);
    expect(apply(mirrorVertical([0, 5]), [7, 3])).toEqual([7, 7]);
  });

  it('scales about a centre', () => {
    expect(apply(scaling(2, [1, 1]), [2, 3])).toEqual([3, 5]);
    expect(apply(about([3, 0, 0, 3, 0, 0], [0, 0]), [1, 2])).toEqual([3, 6]);
  });

  it('writes an SVG matrix and small helpers', () => {
    expect(svgMatrix([1, 0, 0, 1, 2.5, -3])).toBe('matrix(1 0 0 1 2.5 -3)');
    expect(tenth(12.345)).toBe(12.3);
    expect(centre({ minX: 0, maxX: 10, minY: -4, maxY: 2 })).toEqual([5, -1]);
  });
});
