// What an import review shows, and whether a file needs one before it is
// kept: a choice to make (layers, an ambiguous scale), something repaired,
// or something to check (size, open or self-crossing shapes).
import type { ImportLayer, ImportReview, Repairs, SvgScale } from '../api';

/** The scales a pixel-sized SVG can be read at. */
export const SCALES: readonly (readonly [SvgScale, string])[] = [
  ['dpi96', '96 px per inch'],
  ['dpi72', '72 px per inch'],
  ['millimetre', '1 unit = 1 mm'],
];

/** Whether the operator can choose how the file's pixels scale. */
export const scaleChoosable = (review: ImportReview): boolean => !!review.scale && review.scale.units !== 'physical';

/** Whether the file has layers worth choosing between. */
export const layersChoosable = (review: ImportReview): boolean =>
  review.layers.length > 1 || review.layers.some((layer) => layer.hidden);

/** Whether anything about the file should be seen before it is kept. */
export function needsReview(review: ImportReview): boolean {
  const repaired = review.repairs.gaps.length + review.repairs.duplicates.length + review.repairs.mirrored.length > 0;
  return review.scale?.units === 'ambiguous'
    || review.layers.some((layer) => layer.hidden)
    || repaired
    || !!review.size?.tiny || !!review.size?.exceeds_bed
    || review.open.length > 0 || review.crossing.length > 0
    || review.contours === 0;
}

/** The layers to import after turning `name` on or off. */
export function toggleLayer(layers: readonly ImportLayer[], name: string, on: boolean): string[] {
  return layers.filter((layer) => (layer.name === name ? on : layer.imported)).map((layer) => layer.name);
}

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

/** One line for each kind of repair made. */
export function repairLines(repairs: Repairs): string[] {
  const lines: string[] = [];
  if (repairs.gaps.length) {
    const widest = Math.max(...repairs.gaps.map((gap) => gap.size));
    lines.push(`${plural(repairs.gaps.length, 'gap', 'gaps')} closed, the widest ${widest.toFixed(3)} mm`);
  }
  if (repairs.duplicates.length) lines.push(`${plural(repairs.duplicates.length, 'repeated line or arc', 'repeated lines and arcs')} removed`);
  if (repairs.mirrored.length) lines.push(`${plural(repairs.mirrored.length, 'upside-down entity', 'upside-down entities')} mirrored into place`);
  return lines;
}

/** Each contour's role in the preview. */
export function contourKinds(review: ImportReview): ('closed' | 'open' | 'crossing')[] {
  const open = new Set(review.open), crossing = new Set(review.crossing);
  return review.outline.map((_, i) => (crossing.has(i) ? 'crossing' : open.has(i) ? 'open' : 'closed'));
}
