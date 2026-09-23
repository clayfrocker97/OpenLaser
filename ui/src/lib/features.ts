import { quantity } from '../lib/units.svelte';
import { plural } from './format';
// The machining features as the tool bar shows them: which are on, their
// one-line state, and sensible values when switched on.
import type { Bridges, CommonEdges, Cooling, Features, Joints, Kerf, Leads } from '../api';

export interface Tool { id: string; short: string; name: string; desc: string; optional: boolean }

export const TOOLS: Tool[] = [
  { id: 'leads', short: 'Leads', name: 'Entry & exit leads', desc: 'Keep pierce marks off the finished edge.', optional: true },
  { id: 'joints', short: 'Microjoints', name: 'Microjoints', desc: 'Leave small gaps to hold parts in place.', optional: true },
  { id: 'cooling', short: 'Cooling', name: 'Cooling points', desc: 'Pause at corners to reduce heat.', optional: true },
  { id: 'kerf', short: 'Kerf', name: 'Kerf compensation', desc: 'Offset cuts to allow for the beam width.', optional: true },
  { id: 'bridges', short: 'Bridges', name: 'Bridges', desc: 'Join contours with a channel.', optional: true },
  { id: 'common', short: 'Common edges', name: 'Common edges', desc: 'Cut selected shared edges once.', optional: true },
  { id: 'order', short: 'Cut order', name: 'Cutting order', desc: 'Choose the sequence of cuts.', optional: false },
  { id: 'start', short: 'Start & seam', name: 'Start point and seam', desc: 'Set start points, direction, and seam treatment.', optional: false },
];

export const defaultLeads = (): Leads => ({ entry: { shape: 'line', length: 2, radius: 1, angle: 45 }, exit: null, side: 'auto', closed_only: true, overrides: [] });
export const defaultJoints = (): Joints => ({ placement: { count: 2 }, width: 0.4, minimum_size: 40, outer_only: true, open_start: false, behaviour: 'laser_off', slow_speed: null, repierce: false });
export const defaultCooling = (): Cooling => ({ dwell: 300, placement: { automatic: { at_start: true, corners_below: 60 } } });
export const defaultKerf = (): Kerf => ({ width: 0.2, side: 'auto' });
export const defaultBridges = (): Bridges => ({ width: 2, connections: [] });
export const defaultCommon = (contours: number[]): CommonEdges => ({ contours: [...contours], tolerance: 0.01, allow_overcut: false });

export function isOn(features: Features, id: string): boolean {
  switch (id) {
    case 'leads': return features.leads !== null;
    case 'joints': return features.joints !== null;
    case 'cooling': return features.cooling !== null;
    case 'kerf': return features.kerf !== null;
    case 'bridges': return features.bridges !== null;
    case 'common': return features.common !== null;
    default: return true;
  }
}

export function toggled(features: Features, id: string, contours: number[] = []): Features {
  const f = { ...features };
  if (id === 'leads') f.leads = f.leads ? null : defaultLeads();
  if (id === 'joints') f.joints = f.joints ? null : defaultJoints();
  if (id === 'cooling') f.cooling = f.cooling ? null : defaultCooling();
  if (id === 'kerf') f.kerf = f.kerf ? null : defaultKerf();
  if (id === 'bridges') f.bridges = f.bridges ? null : defaultBridges();
  if (id === 'common') f.common = f.common ? null : defaultCommon(contours);
  return f;
}

/** The words the operator reads for a choice the server names by enum. */
export const WORDS: Record<string, string> = {
  keep: 'As drawn', automatic: 'Automatic', manual: 'Manual',
  clockwise: 'Clockwise', counterclockwise: 'Counterclockwise', reverse: 'Reversed',
  as_drawn: 'As drawn', nearest: 'Shortest path', left_to_right: 'Left to right', right_to_left: 'Right to left', bottom_to_top: 'Bottom to top', top_to_bottom: 'Top to bottom',
  line: 'Line', arc: 'Arc', line_arc: 'Line and arc',
};
const title = (s: string) => WORDS[s] ?? s.replace(/_/g, ' ');

export function stateOf(features: Features, id: string): string {
  switch (id) {
    case 'leads': { const l = features.leads; if (!l) return 'Off'; const e = l.entry; return e ? `${title(e.shape)} ${quantity(e.length, 'mm')}` : 'Exit only'; }
    case 'joints': {
      const j = features.joints; if (!j) return 'Off';
      const p = j.placement;
      const n = 'count' in p ? `${p.count} per contour` : 'spacing' in p ? `every ${quantity(p.spacing, 'mm')}` : 'across_x' in p ? `${p.across_x} across X` : 'across_y' in p ? `${p.across_y} across Y` : `${p.manual.length} placed`;
      return `${n} · ${quantity(j.width, 'mm')}`;
    }
    case 'cooling': { const c = features.cooling; if (!c) return 'Off'; return `${(c.dwell / 1000).toFixed(1)} s · ${'manual' in c.placement ? `${c.placement.manual.length} placed` : 'corners'}`; }
    case 'kerf': { const k = features.kerf; return k ? `${quantity(k.width, 'mm')}` : 'Off'; }
    case 'bridges': { const b = features.bridges; return b ? `${b.connections.length} × ${quantity(b.width, 'mm')}` : 'Off'; }
    case 'common': { const c = features.common; return c ? `${plural(c.contours.length, 'contour')} · ${quantity(c.tolerance, 'mm')}` : 'Off'; }
    case 'order': { const o = features.order; return typeof o.strategy === 'string' ? (o.inner_first ? 'Inner first' : title(o.strategy)) : 'Manual'; }
    case 'start': { const s = features.start; const at = typeof s.position === 'string' ? title(s.position) : 'Manual'; return `${s.spots.length ? `${s.spots.length} chosen` : at} · ${title(s.direction)}`; }
    default: return '';
  }
}
