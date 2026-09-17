import type { Preview } from '../api';

type Point = [number, number];
export type PreviewSegment = { a: Point; b: Point; contour: number; length: number };

/** Display geometry only; this animation does not estimate machine timing. */
export function orderSegments(preview: Preview | null): PreviewSegment[] {
  return (preview?.contours ?? []).flatMap((c, contour) => c.paths.flatMap(p => p.points.slice(1).map((point, i) => {
    const a = p.points[i]! as Point, b = point as Point;
    return { a, b, contour, length: Math.hypot(b[0] - a[0], b[1] - a[1]) };
  })));
}

export function orderPosition(segments: PreviewSegment[], progress: number): { point: Point; contour: number } | null {
  let left = Math.max(0, Math.min(1, progress)) * segments.reduce((sum, s) => sum + s.length, 0);
  for (const s of segments) {
    if (left <= s.length && s.length > 0) {
      const t = left / s.length;
      return { point: [s.a[0] + (s.b[0] - s.a[0]) * t, s.a[1] + (s.b[1] - s.a[1]) * t], contour: s.contour };
    }
    left -= s.length;
  }
  const last = segments.at(-1);
  return last ? { point: last.b, contour: last.contour } : null;
}
