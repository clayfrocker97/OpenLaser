// Polylines as SVG paths, and thumbnails fitted into a box. Drawing
// coordinates have Y up; the SVG has Y down, so paths are flipped.
export type Polyline = ReadonlyArray<readonly [number, number]> | number[][];

export function pathOf(points: Polyline, flipY = true): string {
  return points.map((p, i) => `${i === 0 ? 'M' : 'L'}${p[0]!.toFixed(2)} ${(flipY ? -p[1]! : p[1]!).toFixed(2)}`).join('');
}

export interface Box { minX: number; minY: number; maxX: number; maxY: number }

export function boxOf(polylines: Polyline[]): Box | null {
  let box: Box | null = null;
  for (const line of polylines) {
    for (const p of line) {
      if (!box) box = { minX: p[0]!, minY: p[1]!, maxX: p[0]!, maxY: p[1]! };
      else {
        box.minX = Math.min(box.minX, p[0]!); box.maxX = Math.max(box.maxX, p[0]!);
        box.minY = Math.min(box.minY, p[1]!); box.maxY = Math.max(box.maxY, p[1]!);
      }
    }
  }
  return box;
}

/** A viewBox (Y flipped) that fits `box` with a margin, in a w:h aspect. */
export function viewBoxFor(box: Box, w: number, h: number, margin = 0.08): string {
  const bw = Math.max(box.maxX - box.minX, 1e-6), bh = Math.max(box.maxY - box.minY, 1e-6);
  let vw = bw * (1 + 2 * margin), vh = bh * (1 + 2 * margin);
  if (vw / vh < w / h) vw = vh * (w / h); else vh = vw * (h / w);
  const cx = (box.minX + box.maxX) / 2, cy = -(box.minY + box.maxY) / 2;
  return `${cx - vw / 2} ${cy - vh / 2} ${vw} ${vh}`;
}

/** A box from the server's bounds. */
export const boxOfBounds = (bounds: { min: { x: number; y: number }; max: { x: number; y: number } }): Box => ({ minX: bounds.min.x, minY: bounds.min.y, maxX: bounds.max.x, maxY: bounds.max.y });
