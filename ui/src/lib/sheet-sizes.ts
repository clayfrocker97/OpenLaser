// Common sheet sizes, and fitting a size to the bed.
import { toDisplay, unitLabel, units } from './units.svelte';
import type { Bounds } from '../api';

/** A sheet size in millimetres, long side first as sold. */
export interface SheetSize { label: string; long: number; short: number }

/** Stock sizes sold by metal suppliers, metric then imperial. */
export const COMMON_SHEETS: SheetSize[] = [
  { label: '500 × 1000', long: 1000, short: 500 },
  { label: '1000 × 1000', long: 1000, short: 1000 },
  { label: '1000 × 2000', long: 2000, short: 1000 },
  { label: '1250 × 2500', long: 2500, short: 1250 },
  { label: '1500 × 3000', long: 3000, short: 1500 },
  { label: '2000 × 4000', long: 4000, short: 2000 },
  { label: '2 × 4 ft', long: 1219.2, short: 609.6 },
  { label: '4 × 4 ft', long: 1219.2, short: 1219.2 },
  { label: '4 × 8 ft', long: 2438.4, short: 1219.2 },
  { label: '5 × 10 ft', long: 3048, short: 1524 },
];

/** Width and height on the bed: the long side follows the bed's long side. */
export function oriented(long: number, short: number, bed: { width: number; height: number } | null): [number, number] {
  return !bed || bed.width >= bed.height ? [long, short] : [short, long];
}

/** Whether a sheet fits on the bed in its given orientation. */
export const fits = ([width, height]: [number, number], bed: { width: number; height: number } | null): boolean =>
  !bed || (width <= bed.width + 1e-6 && height <= bed.height + 1e-6);

/** A sheet side the way stock is sold: whole millimetres, or inches to 0.01. */
export const side = (mm: number): string =>
  Number(toDisplay(mm, 'mm').toFixed(units.system === 'imperial' ? 2 : 0)).toLocaleString('en-US');

/** A sheet's width by height, with the unit. */
export const sheetLabel = (width: number, height: number): string => `${side(width)} × ${side(height)} ${unitLabel('mm')}`;
/** A sheet's size from its bounds, such as a remnant's. */
export const boundsLabel = (b: Bounds): string => sheetLabel(b.max.x - b.min.x, b.max.y - b.min.y);
