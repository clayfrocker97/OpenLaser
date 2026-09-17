import type { LeadOverride, Transform } from '../api';

type Placed = { source: number; transform: Transform };
export type PasteDirection = 'right' | 'left' | 'up' | 'down';
export interface PasteSettings { count: number; gap: number; direction: PasteDirection }
export interface CopiedShapes {
  part: string;
  contours: Placed[];
  leads: LeadOverride[];
  grouping: number[][];
  width: number;
  height: number;
  offset: [number, number];
}

/** One add request gives the whole batch one undo step. Indices are batch-relative. */
export function pasteBatch(copied: CopiedShapes, settings: PasteSettings, placedCount: number) {
  if (!Number.isInteger(settings.count) || settings.count < 1 || settings.count > 500) throw Error('Enter a whole copy count from 1 to 500.');
  if (!Number.isFinite(settings.gap) || settings.gap < 0) throw Error('Use a finite gap of zero or more.');
  if (!copied.contours.length) throw Error('Copy shapes from the drawing first.');
  // Same layout limit as openlaser_core::geometry::MAX_PLACED_CONTOURS.
  if (copied.contours.length * settings.count > 100000 - placedCount) throw Error('This paste would exceed 100,000 contours. Use fewer copies.');
  const steps: Record<PasteDirection, [number, number]> = {
    right: [copied.width + settings.gap, 0], left: [-copied.width - settings.gap, 0],
    up: [0, copied.height + settings.gap], down: [0, -copied.height - settings.gap],
  };
  const [dx, dy] = steps[settings.direction];
  const contours: Placed[] = [], lead_overrides: LeadOverride[] = [], grouping: number[][] = [];
  let [x, y] = copied.offset;
  for (let i = 0; i < settings.count; i++) {
    x += dx; y += dy;
    const first = contours.length;
    for (const contour of copied.contours) {
      const transform: Transform = [...contour.transform];
      transform[4] += x; transform[5] += y;
      contours.push({ source: contour.source, transform });
    }
    for (const lead of copied.leads) lead_overrides.push({ ...lead, location: { ...lead.location, contour: first + lead.location.contour } });
    for (const group of copied.grouping) grouping.push(group.map(index => first + index));
  }
  return { contours, lead_overrides, grouping, offset: [x, y] as [number, number] };
}
