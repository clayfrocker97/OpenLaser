import type { Checkpoint, Compiled, RecoveryStep } from '../api';

/** Project a touch onto an original cutting path, in drawing coordinates. */
export function restartAt(compiled: Pick<Compiled, 'moves'>, steps: readonly RecoveryStep[], point: readonly [number, number], radius: number): Checkpoint | null {
  const paths = new Map<number, Array<Array<[number, number]>>>();
  for (const move of compiled.moves) {
    if (move.pass === null || move.kind === 'travel' || move.kind === 'cleaning' || steps[move.pass]?.status === 'skipped') continue;
    const runs = paths.get(move.pass) ?? [];
    runs.push(move.points); paths.set(move.pass, runs);
  }
  let best: { distance: number; checkpoint: Checkpoint; rank: number } | null = null;
  for (const [pass, runs] of paths) {
    const segments = runs.flatMap(path => path.slice(1).map((b, i) => ({ a: path[i]!, b, length: Math.hypot(b[0] - path[i]![0], b[1] - path[i]![1]) })));
    const total = segments.reduce((sum, s) => sum + s.length, 0);
    let travelled = 0;
    for (const { a, b, length } of segments) {
      if (!length) continue;
      const dx = b[0] - a[0], dy = b[1] - a[1];
      const t = Math.max(0, Math.min(1, ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / (length * length)));
      const distance = Math.hypot(point[0] - a[0] - t * dx, point[1] - a[1] - t * dy);
      const rank = Number(steps[pass]?.pass.kind === 'cut') * 2 + Number(steps[pass]?.status !== 'completed');
      if (distance <= radius && (!best || distance < best.distance - 1e-8 || (Math.abs(distance - best.distance) <= 1e-8 && rank > best.rank))) best = { distance, rank, checkpoint: { pass, fraction: total ? (travelled + t * length) / total : 0 } };
      travelled += length;
    }
  }
  return best?.checkpoint ?? null;
}
