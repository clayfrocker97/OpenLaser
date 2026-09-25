// Which sheets a nesting search fills, in the order the operator lists
// them. Nothing is suggested: the operator chooses every sheet.
import type { LaserMode, SheetView, StockItem, StockSource } from '../api';

type Recipe = { laser: LaserMode; name: string; thickness_mm: number };
type Material = { laser: LaserMode; material: string; thickness_mm: number };

/** Sheets suit a recipe with the same laser, material and thickness, and anything before a
 *  recipe is chosen; the server applies the same rule. */
export const suits = (stock: Material, recipe: Recipe | null | undefined): boolean =>
  !recipe || stock.laser === recipe.laser && Math.abs(stock.thickness_mm - recipe.thickness_mm) < 1e-6
  && stock.material.trim().toLowerCase() === recipe.name.trim().toLowerCase();

export const remnantMaterial = (sheet: SheetView): Material =>
  ({ laser: sheet.mode, material: sheet.material, thickness_mm: sheet.thickness_mm });

const area = (points: Array<[number, number]>): number =>
  Math.abs(points.reduce((sum, [x, y], i) => {
    const [nx, ny] = points[(i + 1) % points.length]!;
    return sum + x * ny - nx * y;
  }, 0)) / 2;

/** Material left on a remnant: its outline less what was already cut. */
export const usable = (sheet: SheetView): number =>
  Math.max(0, area(sheet.outline) - sheet.cutouts.reduce((sum, c) => sum + area(c), 0));

/**
 * A plan as the library stands now, in the operator's order: sheets that are
 * gone are left out, each rack entry, remnant and new size is listed once,
 * and a rack count is at most what is on hand.
 */
export function current(plan: StockSource[], rack: StockItem[], remnants: SheetView[]): StockSource[] {
  const out: StockSource[] = [];
  for (const source of plan) {
    if (source.kind === 'remnant') {
      if (remnants.some((r) => r.id === source.id) && !out.some((o) => o.kind === 'remnant' && o.id === source.id)) out.push(source);
    } else if (source.kind === 'stock') {
      const onHand = rack.find((i) => i.id === source.id)?.quantity ?? 0;
      const listed = out.findIndex((o) => o.kind === 'stock' && o.id === source.id);
      const before = listed >= 0 ? (out[listed] as { count: number }).count : 0;
      const entry = { ...source, count: Math.min(onHand, before + source.count) };
      if (listed >= 0) out[listed] = entry;
      else if (entry.count > 0) out.push(entry);
    } else {
      // A size listed twice is one row of both counts.
      const listed = out.findIndex((o) => o.kind === 'sheet' && o.width === source.width && o.height === source.height);
      const before = listed >= 0 ? out[listed] as typeof source : null;
      if (!before) out.push(source);
      else out[listed] = { ...before, count: before.count === null || source.count === null ? null : before.count + source.count };
    }
  }
  return out;
}

/** One more of a listed sheet: a new sheet always, a rack sheet while more are on hand. */
export function oneMore(source: StockSource | undefined, rack: StockItem[]): StockSource | null {
  if (source?.kind === 'sheet') return { ...source, count: (source.count ?? 1) + 1 };
  if (source?.kind === 'stock') {
    const onHand = rack.find((i) => i.id === source.id)?.quantity ?? 0;
    return source.count < onHand ? { ...source, count: source.count + 1 } : null;
  }
  return null;
}
