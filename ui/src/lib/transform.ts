// Uniform similarity transforms, as the server keeps them: [a b c d e f],
// mapping (x, y) to (a·x + c·y + e, b·x + d·y + f), as in SVG's matrix().
import type { Transform } from '../api';
import type { Box } from './svg';

export type Point = [number, number];

/** `a` applied after `b`: the transform that first does `b`, then `a`. */
export const after = (a: Transform, b: Transform): Transform => [
  a[0] * b[0] + a[2] * b[1],
  a[1] * b[0] + a[3] * b[1],
  a[0] * b[2] + a[2] * b[3],
  a[1] * b[2] + a[3] * b[3],
  a[0] * b[4] + a[2] * b[5] + a[4],
  a[1] * b[4] + a[3] * b[5] + a[5],
];

/** A move by `dx`, `dy`. */
export const translate = (dx: number, dy: number): Transform => [1, 0, 0, 1, dx, dy];

/** `m` taken about the point `c` instead of the origin. */
export const about = (m: Transform, c: Point): Transform => after(translate(c[0], c[1]), after(m, translate(-c[0], -c[1])));

/** A counterclockwise turn by `degrees` about `c`. */
export const rotation = (degrees: number, c: Point): Transform => {
  const r = (degrees * Math.PI) / 180;
  return about([Math.cos(r), Math.sin(r), -Math.sin(r), Math.cos(r), 0, 0], c);
};

/** A mirror across the vertical line through `c`. */
export const mirror = (c: Point): Transform => about([-1, 0, 0, 1, 0, 0], c);

/** A mirror across the horizontal line through `c`. */
export const mirrorVertical = (c: Point): Transform => about([1, 0, 0, -1, 0, 0], c);

/** A uniform scale by `factor` about `c`. */
export const scaling = (factor: number, c: Point): Transform => about([factor, 0, 0, factor, 0, 0], c);

/** Where `m` takes the point `p`. */
export const apply = (m: Transform, p: Point): Point => [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]];

/** The SVG `transform` attribute for `m`. */
export const svgMatrix = (m: Transform): string => `matrix(${m.join(' ')})`;


/** The centre of a box. */
export const centre = (b: Box): Point => [(b.minX + b.maxX) / 2, (b.minY + b.maxY) / 2];
