// The canvas is in machine coordinates: the bed stays put, the head is
// where the controller says, and the part is drawn where it will be cut.
// The server lands a sheet with its anchor point at the front-left of the
// travel; from then on the operator moves it, or puts its anchor point
// at the head or a typed spot. The travel comes from the bindings once
// connected and from the loaded backup before that.
import type { Document } from '../api';
import type { Box } from './svg';

export interface Frame {
  /** The machine's travel. */
  bed: Box | null;
  /** The head, while connected. */
  head: [number, number] | null;
  /** Where the part's anchor point lies, once prepared. */
  origin: [number, number] | null;
  /** What the machine adds to a drawing coordinate, once the sheet is placed. */
  zero: [number, number];
}

export function frameOf(doc: Document): Frame {
  const extent = doc.bindings?.extent ?? doc.files.extent ?? null;
  const bed = extent ? { minX: extent[0][0], maxX: extent[0][1], minY: extent[1][0], maxY: extent[1][1] } : null;
  const position = doc.machine.feedback?.position_mm;
  const head: [number, number] | null = position ? [position[0], position[1]] : null;
  return { bed, head, origin: doc.draft?.origin ?? null, zero: doc.draft?.zero ?? [0, 0] };
}
