import { describe, expect, it } from 'vitest';
import type { Lead, Leads, PreviewContour } from '../api';
import { draggedLead, leadHandlesOf } from './lead-handles';

const line = (length: number, angle = 90): Lead => ({ shape: 'line', length, angle, radius: 1 });
const leads = (side: Leads['side'] = 'outside'): Leads => ({ entry: line(2), exit: line(1), closed_only: true, side, overrides: [] });
// A counterclockwise square with a lead-in from (-2, 0) and a lead-out to (0, -1).
const square = (source = 0): PreviewContour => ({
  sources: [source], lead_target: { contour: source, fraction: 0 }, matching_contour: null, depth: 0,
  paths: [
    { kind: 'lead_in', points: [[-2, 0], [0, 0]] },
    { kind: 'cut', points: [[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]] },
    { kind: 'lead_out', points: [[0, 0], [0, -1]] },
  ],
}) as unknown as PreviewContour;

describe('lead handles', () => {
  it('puts a handle at the free end of each straight lead on selected contours', () => {
    const handles = leadHandlesOf([square(0), square(1)], new Set([0]), leads());
    expect(handles.map(h => [h.which, h.anchor, h.end])).toEqual([
      ['entry', [0, 0], [-2, 0]],
      ['exit', [0, 0], [0, -1]],
    ]);
    expect(handles[0]!.side).toBe(-1);
    expect(leadHandlesOf([square()], new Set([0]), leads('inside'))[0]!.side).toBe(1);
  });

  it('skips arcs, switched-off leads and contours without a target', () => {
    const arcs = { ...leads(), entry: { ...line(2), shape: 'arc' as const }, exit: null };
    expect(leadHandlesOf([square()], new Set([0]), arcs)).toEqual([]);
    expect(leadHandlesOf([{ ...square(), lead_target: null }], new Set([0]), leads())).toEqual([]);
  });

  it('stretches and turns a dragged lead about its anchor', () => {
    const [entry] = leadHandlesOf([square()], new Set([0]), leads());
    const same = draggedLead(entry!, [-2, 0]);
    expect(same.length).toBeCloseTo(2);
    expect(same.angle).toBeCloseTo(90);
    const turned = draggedLead(entry!, [0, 4]);
    expect(turned.length).toBeCloseTo(4);
    // A quarter turn clockwise about the anchor, on an outside lead-in.
    expect(turned.angle).toBeCloseTo(0);
  });
});
