import { boxOf, pathOf, type Box, type Polyline } from './svg';
import type { PreviewContour, Transform } from '../api';

// Preview arrays are immutable snapshots from the server. The weak caches
// disappear with the snapshot; cut geometry is never rounded or modified here.
const paths = new WeakMap<Polyline, string>();
const hits = new WeakMap<PreviewContour, string>();
const contourBounds = new WeakMap<PreviewContour, Box | null>();
export function drawingPath(points: Polyline): string {
  let path = paths.get(points);
  if (path === undefined) { path = pathOf(points, false); paths.set(points, path); }
  return path;
}
export function hitPath(contour: PreviewContour): string {
  let path = hits.get(contour);
  if (path === undefined) { path = contour.paths.map(p => drawingPath(p.points)).join(' '); hits.set(contour, path); }
  return path;
}
function boundsOfContour(contour: PreviewContour): Box | null {
  if (!contourBounds.has(contour)) contourBounds.set(contour, boxOf(contour.paths.map(p => p.points)));
  return contourBounds.get(contour) ?? null;
}
/** The shape whose line runs nearest `at`, within `tolerance`, so a tap
 *  on a hole inside a ring takes the hole, whichever is drawn on top. */
export function nearestShape(
  shapes: Map<number, PreviewContour[]>, index: BoundsIndex, at: [number, number], tolerance: number,
  shown: (contour: PreviewContour) => boolean = () => true,
): { group: number; contour: PreviewContour } | null {
  const [x, y] = at;
  let best: { group: number; contour: PreviewContour } | null = null;
  let nearest = tolerance;
  const box = { minX: x - tolerance, maxX: x + tolerance, minY: y - tolerance, maxY: y + tolerance };
  for (const group of index.query(box)) {
    for (const contour of shapes.get(group) ?? []) {
      if (!shown(contour)) continue;
      for (const path of contour.paths) {
        for (let i = 1; i < path.points.length; i++) {
          const [ax, ay] = path.points[i - 1]!, [bx, by] = path.points[i]!;
          const dx = bx - ax, dy = by - ay, length = dx * dx + dy * dy;
          const t = length ? Math.max(0, Math.min(1, ((x - ax) * dx + (y - ay) * dy) / length)) : 0;
          const distance = Math.hypot(x - (ax + t * dx), y - (ay + t * dy));
          if (distance < nearest) { nearest = distance; best = { group, contour }; }
        }
      }
    }
  }
  return best;
}

/** The line a tap takes: the nearest within `halo` of it. Inside a closed
 *  shape the halos grow until they meet, so the middle of a hole takes the
 *  hole and a point between a hole and its outline takes whichever is
 *  nearer; open space outside every shape keeps the fixed halo. */
export function pickLine(
  shapes: Map<number, PreviewContour[]>, index: BoundsIndex, at: [number, number], halo: number,
  shown: (contour: PreviewContour) => boolean = () => true,
): { group: number; contour: PreviewContour } | null {
  const near = nearestShape(shapes, index, at, halo, shown);
  if (near) return near;
  // The innermost shape around the point bounds how far its nearest line can be.
  const [x, y] = at;
  let reach = Infinity;
  for (const group of index.query({ minX: x, maxX: x, minY: y, maxY: y })) {
    for (const contour of shapes.get(group) ?? []) {
      const bounds = boundsOfContour(contour);
      if (!bounds || !shown(contour) || x < bounds.minX || x > bounds.maxX || y < bounds.minY || y > bounds.maxY) continue;
      const size = Math.hypot(bounds.maxX - bounds.minX, bounds.maxY - bounds.minY);
      if (size < reach && contour.paths.some((p) => p.kind === 'cut' && encloses(p.points, x, y))) reach = size;
    }
  }
  return reach === Infinity ? null : nearestShape(shapes, index, at, reach, shown);
}

/** Whether a polyline has a point inside the box or a segment through it. */
function crosses(points: Polyline, box: Box): boolean {
  const inside = (x: number, y: number) => x >= box.minX && x <= box.maxX && y >= box.minY && y <= box.maxY;
  for (let i = 0; i < points.length; i++) {
    const [x, y] = points[i]!;
    if (inside(x, y)) return true;
    if (i && segmentThrough(points[i - 1]!, points[i]!, box)) return true;
  }
  return false;
}

/** Whether a segment passes through the box, clipped against its sides. */
function segmentThrough(a: readonly number[], b: readonly number[], box: Box): boolean {
  const [ax = 0, ay = 0] = a, [bx = 0, by = 0] = b;
  let t0 = 0, t1 = 1;
  const dx = bx - ax, dy = by - ay;
  for (const [p, q] of [[-dx, ax - box.minX], [dx, box.maxX - ax], [-dy, ay - box.minY], [dy, box.maxY - ay]] as const) {
    if (p === 0) { if (q < 0) return false; continue; }
    const t = q / p;
    if (p < 0) { if (t > t1) return false; if (t > t0) t0 = t; } else { if (t < t0) return false; if (t < t1) t1 = t; }
  }
  return t0 <= t1;
}

/** Whether a closed line runs around the point, by the crossings of a ray. */
function encloses(points: Polyline, x: number, y: number): boolean {
  const first = points[0], last = points[points.length - 1];
  if (!first || !last || points.length < 3 || Math.hypot(first[0] - last[0], first[1] - last[1]) > 1e-3) return false;
  let inside = false;
  for (let i = 0, j = points.length - 1; i < points.length; j = i++) {
    const [xi, yi] = points[i]!, [xj, yj] = points[j]!;
    if ((yi > y) !== (yj > y) && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

/** A marquee touches any member of a group; empty space between members does not count. */
export function marqueeGroups(shapes: Map<number, PreviewContour[]>, index: BoundsIndex, box: Box): number[] {
  return index.query(box).filter(g => shapes.get(g)?.some(c => {
    const bounds = boundsOfContour(c);
    return bounds !== null && overlaps(bounds, box);
  })).sort((a, b) => a - b);
}
/** Every single-source contour whose line runs through a marquee, with its
 *  group: the shapes a box takes one by one, such as the letters of some
 *  text, even partly covered, and not the plate whose outline only goes
 *  around the box. */
export function marqueeContours(
  shapes: Map<number, PreviewContour[]>, index: BoundsIndex, box: Box,
  shown: (contour: PreviewContour) => boolean = () => true,
): Array<{ group: number; source: number }> {
  return index.query(box).flatMap((group) => (shapes.get(group) ?? []).flatMap((c) => {
    const bounds = boundsOfContour(c);
    const source = c.sources.length === 1 ? c.sources[0]! : null;
    const touched = bounds !== null && overlaps(bounds, box) && c.paths.some((p) => crosses(p.points, box));
    return source !== null && touched && shown(c) ? [{ group, source }] : [];
  }));
}
export function boundsOfShapes(shapes: Map<number, PreviewContour[]>): Map<number, Box> {
  const bounds = new Map<number, Box>();
  for (const [g, contours] of shapes) {
    // Include entry and exit paths as well as the part itself.
    const box = unionBounds(contours.map(c => boundsOfContour(c) ?? undefined));
    if (box) bounds.set(g, box);
  }
  return bounds;
}
export function unionBounds(boxes: Iterable<Box | undefined>): Box | null {
  let result: Box | null = null;
  for (const box of boxes) {
    if (!box) continue;
    if (!result) result = { ...box };
    else {
      result.minX = Math.min(result.minX, box.minX); result.minY = Math.min(result.minY, box.minY);
      result.maxX = Math.max(result.maxX, box.maxX); result.maxY = Math.max(result.maxY, box.maxY);
    }
  }
  return result;
}
/** Exact for translation, axis-aligned scale and reflection. Rotation requires vertices. */
export function axisAlignedBounds(box: Box | null, m: Transform): Box | null {
  if (!box || m[1] !== 0 || m[2] !== 0) return null;
  const x1 = box.minX * m[0] + m[4], x2 = box.maxX * m[0] + m[4];
  const y1 = box.minY * m[3] + m[5], y2 = box.maxY * m[3] + m[5];
  return { minX: Math.min(x1, x2), minY: Math.min(y1, y2), maxX: Math.max(x1, x2), maxY: Math.max(y1, y2) };
}
export function transformedBounds(lines: Iterable<Polyline>, m: Transform): Box | null {
  let box: Box | null = null;
  for (const points of lines) for (const p of points) {
    const x = m[0] * p[0]! + m[2] * p[1]! + m[4];
    const y = m[1] * p[0]! + m[3] * p[1]! + m[5];
    if (!box) box = { minX: x, minY: y, maxX: x, maxY: y };
    else {
      box.minX = Math.min(box.minX, x); box.minY = Math.min(box.minY, y);
      box.maxX = Math.max(box.maxX, x); box.maxY = Math.max(box.maxY, y);
    }
  }
  return box;
}

export function overlaps(a: Box, b: Box): boolean {
  return a.minX <= b.maxX && a.maxX >= b.minX && a.minY <= b.maxY && a.maxY >= b.minY;
}
/** Conservative camera bounds in a moving selection's original coordinates. */
export function inverseBounds(box: Box, m: Transform): Box | null {
  const det = m[0] * m[3] - m[1] * m[2];
  if (!Number.isFinite(det) || Math.abs(det) < 1e-15) return null;
  const inverse: Transform = [m[3] / det, -m[1] / det, -m[2] / det, m[0] / det,
    (m[2] * m[5] - m[3] * m[4]) / det, (m[1] * m[4] - m[0] * m[5]) / det];
  return transformedBounds([[[box.minX, box.minY], [box.maxX, box.minY], [box.maxX, box.maxY], [box.minX, box.maxY]]], inverse);
}

type Entry = { group: number; box: Box };
type Node = { box: Box; entries: Entry[] } | { box: Box; left: Node; right: Node };
/** A balanced bounds tree rebuilt only when the prepared geometry changes. */
export class BoundsIndex {
  private root: Node | null;
  constructor(bounds: Map<number, Box>) {
    const build = (entries: Entry[]): Node | null => {
      const box = unionBounds(entries.map(e => e.box));
      if (!box) return null;
      if (entries.length <= 8) return { box, entries };
      const wide = box.maxX - box.minX >= box.maxY - box.minY;
      entries.sort((a, b) => wide ? a.box.minX + a.box.maxX - b.box.minX - b.box.maxX : a.box.minY + a.box.maxY - b.box.minY - b.box.maxY);
      const mid = Math.floor(entries.length / 2);
      return { box, left: build(entries.slice(0, mid))!, right: build(entries.slice(mid))! };
    };
    this.root = build([...bounds].map(([group, box]) => ({ group, box })));
  }
  query(box: Box): number[] {
    const result: number[] = [];
    const visit = (node: Node | null) => {
      if (!node || !overlaps(node.box, box)) return;
      if ('entries' in node) {
        for (const entry of node.entries) if (overlaps(entry.box, box)) result.push(entry.group);
      } else { visit(node.left); visit(node.right); }
    };
    visit(this.root);
    return result;
  }
}
