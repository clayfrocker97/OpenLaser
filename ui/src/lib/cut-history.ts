import type { Move, RecoveryStep } from '../api';

/** Display only the path intervals confirmed by the retained execution. */
export function cutHistory(moves: Move[], steps: Pick<RecoveryStep, 'executed'>[]): number[][][] {
  const paths: number[][][] = [];
  const grouped = new Map<number, Move[]>();
  for (const move of moves) {
    if (move.pass === null || ['travel', 'cleaning'].includes(move.kind)) continue;
    const list = grouped.get(move.pass) ?? [];
    list.push(move);
    grouped.set(move.pass, list);
  }
  for (const [pass, parts] of grouped) {
    const ranges = steps[pass]?.executed ?? [];
    if (!ranges.length) continue;
    const length = parts.reduce((total, move) => total + move.points.slice(1).reduce((sum, p, i) => sum + Math.hypot(p[0]! - move.points[i]![0]!, p[1]! - move.points[i]![1]!), 0), 0);
    for (const [from, to] of ranges) {
      let offset = 0;
      for (const move of parts) {
        const points: number[][] = [];
        for (let i = 1; i < move.points.length; i++) {
          const a = move.points[i - 1]!, b = move.points[i]!;
          const span = Math.hypot(b[0]! - a[0]!, b[1]! - a[1]!);
          const lo = Math.max(offset, from * length), hi = Math.min(offset + span, to * length);
          if (hi > lo && span > 0) {
            const at = (distance: number) => [a[0]! + (b[0]! - a[0]!) * (distance - offset) / span, a[1]! + (b[1]! - a[1]!) * (distance - offset) / span];
            if (!points.length) points.push(at(lo));
            points.push(at(hi));
          }
          offset += span;
        }
        if (points.length > 1) paths.push(points);
      }
    }
  }
  return paths;
}
