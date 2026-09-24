// Which sheets a nesting search fills, in what order, and the suggestion
// made before the operator changes it.
import type { DraftView, LaserMode, SheetView, StockItem, StockSource } from '../api';

type Recipe = { laser: LaserMode; name: string; thickness_mm: number };
type Material = { laser: LaserMode; material: string; thickness_mm: number };

/** Room lost between parts and at the edges, as a factor on part area. */
const PACKING = 1.3;
/** A remnant too small for the whole job is still worth loading when it takes this share. */
const WORTH_LOADING = 0.25;

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

/** Sheet area the parts need: one of each part's box, the selected part repeated, plus room between. */
export function needed(draft: Pick<DraftView, 'preview' | 'groups'>, copies: number): number {
  const outer = draft.preview?.contours.filter((c) => c.closed && c.depth === 0) ?? [];
  const set = outer.reduce((sum, c) => {
    const points = c.paths.flatMap((p) => p.points);
    if (!points.length) return sum;
    const xs = points.map((p) => p[0]), ys = points.map((p) => p[1]);
    return sum + (Math.max(...xs) - Math.min(...xs)) * (Math.max(...ys) - Math.min(...ys));
  }, 0);
  const each = set / Math.max(1, draft.groups.length);
  return (set + each * Math.max(0, copies - 1)) * PACKING;
}

const RANK = { remnant: 0, stock: 1, sheet: 2 } as const;
/** Remnants fill first, then sheets on the rack, then new sheets. */
export const ordered = (plan: StockSource[]): StockSource[] =>
  plan.map((s, i) => [s, i] as const).sort((a, b) => RANK[a[0].kind] - RANK[b[0].kind] || a[1] - b[1]).map(([s]) => s);

/**
 * The suggested sheets, as an operator would pick them: the smallest remnant
 * that holds the whole job, keeping larger ones for larger jobs; failing
 * that, the largest remnant when it takes a fair share. Then the matching
 * sheets on the rack, largest first, and new sheets for whatever is left.
 * An unused source costs nothing: a sheet opens only when parts go on it.
 */
export function suggestStock(o: {
  need: number; remnants: SheetView[]; rack: StockItem[]; fallback: [number, number]; chosen?: string;
}): StockSource[] {
  const plan: StockSource[] = [];
  const room = o.remnants.map((sheet) => ({ sheet, room: usable(sheet) })).sort((a, b) => a.room - b.room);
  const remnant = o.chosen ? room.find((r) => r.sheet.id === o.chosen)
    : room.find((r) => r.room >= o.need) ?? room.filter((r) => r.room >= o.need * WORTH_LOADING).at(-1);
  if (remnant) plan.push({ kind: 'remnant', id: remnant.sheet.id });
  const rack = o.rack.filter((i) => i.quantity > 0).sort((a, b) => b.width_mm * b.height_mm - a.width_mm * a.height_mm);
  for (const item of rack) plan.push({ kind: 'stock', id: item.id, count: item.quantity });
  plan.push({ kind: 'sheet', width: o.fallback[0], height: o.fallback[1], count: null });
  return plan;
}

/**
 * A plan as the library stands now: sheets that are gone are left out, each
 * rack entry and remnant is listed once, and a rack count is at most what is
 * on hand.
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
    } else if (!out.some((o) => o.kind === 'sheet' && o.width === source.width && o.height === source.height)) {
      out.push(source);
    }
  }
  return ordered(out);
}
