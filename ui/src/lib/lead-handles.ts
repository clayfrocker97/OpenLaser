// The drag handles on a selection's straight leads: one at the free end of
// each lead-in and lead-out, turning and stretching it about where it meets
// the contour.
import type { Lead, Leads, PreviewContour, Spot } from '../api';
import { leadLookup, type LeadRole } from './lead-overrides';
import type { Point } from './transform';

export type LeadHandle = {
  /** The contour this one copies, so a drag edits every copy's lead alike. */
  matching: number | null;
  which: LeadRole;
  target: Spot;
  definition: Lead;
  /** Where the lead meets the contour. */
  anchor: Point;
  /** The lead's free end, where the handle is drawn. */
  end: Point;
  /** 1 or -1: which way a positive angle turns this lead. */
  side: number;
};

/** Twice the signed area of a ring; positive when it runs counterclockwise. */
const signedArea = (ring: number[][]): number =>
  ring.reduce((sum, a, i) => { const b = ring[(i + 1) % ring.length]!; return sum + a[0]! * b[1]! - b[0]! * a[1]!; }, 0);

/** The handles for every straight lead on the contours drawn from `chosen`. */
export function leadHandlesOf(contours: readonly PreviewContour[], chosen: ReadonlySet<number>, leads: Leads | null): LeadHandle[] {
  const definitionFor = leadLookup(leads);
  return contours.filter(c => c.sources.some(i => chosen.has(i))).flatMap(c => {
    const target = c.lead_target;
    if (!target) return [];
    const body = c.paths.filter(p => p.kind !== 'lead_in' && p.kind !== 'lead_out').flatMap(p => p.points);
    const inside = leads?.side === 'inside' || (leads?.side === 'auto' && c.depth % 2 === 1);
    const side = (signedArea(body) >= 0) === inside ? 1 : -1;
    return (['entry', 'exit'] as const).flatMap((which): LeadHandle[] => {
      const definition = definitionFor(target, which);
      if (definition?.shape !== 'line') return [];
      const path = c.paths.find(p => p.kind === (which === 'entry' ? 'lead_in' : 'lead_out'));
      if (!path || path.points.length < 2) return [];
      const first = path.points[0]! as Point;
      const last = path.points.at(-1)! as Point;
      const [anchor, end] = which === 'entry' ? [last, first] : [first, last];
      return [{ matching: c.matching_contour, which, target, definition, side, anchor, end }];
    });
  });
}

/** The lead a handle dragged to `point` describes. */
export function draggedLead(handle: LeadHandle, point: Point): Lead {
  const [x, y] = handle.anchor;
  const turn = Math.atan2(point[1] - y, point[0] - x) - Math.atan2(handle.end[1] - y, handle.end[0] - x);
  const sign = handle.which === 'entry' ? -handle.side : handle.side;
  return {
    ...handle.definition,
    length: Math.hypot(point[0] - x, point[1] - y),
    angle: handle.definition.angle + turn * 180 / Math.PI * sign,
  };
}
