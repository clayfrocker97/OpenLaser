// The canvas layers, with the prototype's names, colours and dashes: one
// entry per path kind the server labels, plus the pierce marks and the
// cut history. The colours live in app.css beside the path rules.
import type { PathKind } from '../api';

export type Layer = PathKind | 'pierce' | 'done';

export const LAYERS: Array<[Layer, string]> = [
  ['travel', 'Move'],
  ['pierce', 'Pierce'],
  ['cut', 'Cut'],
  ['film', 'Film'],
  ['lead_in', 'Lead in'],
  ['lead_out', 'Lead out'],
  ['joint', 'Microjoint'],
  ['cleaning', 'Clean'],
  ['cooling', 'Dwell'],
  ['done', 'Cut history'],
];
