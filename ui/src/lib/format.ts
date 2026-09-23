// Small formatters shared by the pages.
import type { LaserMode, RecipeView } from '../api';
import { distance, quantity, unitLabel } from './units.svelte';
import { plain } from './plain';

export const fmt = (n: number, digits = 2): string => n.toFixed(digits);

/** A vendor value, stored with every digit of its double, shown to two decimals. */
export const value = (text: string | null | undefined): string => {
  if (text === null || text === undefined) return '—';
  const n = Number(text);
  return Number.isFinite(n) ? String(Math.round(n * 100) / 100) : text;
};

/**
 * A count with its noun, singular for exactly one: `plural(1, 'path')` is
 * "1 path", `plural(3, 'path')` "3 paths", `plural(2, 'cut area')` "2 cut areas".
 */
export const plural = (count: number, singular: string, many = `${singular}s`): string => `${count} ${count === 1 ? singular : many}`;

export const laserLabel = (laser: LaserMode | null | undefined): string => (laser === 'co2' ? 'CO₂' : laser === 'fiber' ? 'Fiber' : '—');

export const recipeLabel = (recipe: Pick<RecipeView, 'name' | 'thickness_mm' | 'gas'> | null | undefined): string =>
  recipe ? `${recipe.name} · ${quantity(recipe.thickness_mm, 'mm')} · ${recipe.gas}` : 'No material';

export const seconds = (s: number): string => {
  const whole = Math.max(0, Math.round(s));
  const m = Math.floor(whole / 60);
  return m > 0 ? `${m} m ${whole % 60} s` : `${whole} s`;
};

export const ago = (epochSeconds: number): string => {
  const delta = Math.max(0, Date.now() / 1000 - epochSeconds);
  if (delta < 60) return 'just now';
  if (delta < 3600) return `${Math.floor(delta / 60)} min ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)} h ago`;
  if (delta < 86400 * 14) return `${Math.floor(delta / 86400)} d ago`;
  return `${Math.floor(delta / (86400 * 7))} w ago`;
};

export const size = (bounds: { min: { x: number; y: number }; max: { x: number; y: number } } | null): string =>
  bounds ? `${distance(bounds.max.x - bounds.min.x, 0)} × ${distance(bounds.max.y - bounds.min.y, 0)} ${unitLabel('mm')}` : '—';

/** The original technical text of recent plain messages, for the toast's Details. */
const originals = new Map<string, string>();
const ORIGINALS_KEPT = 20;

/** Reports a refusal in plain words; the original stays behind `technicalDetail`. */
export const explain = (error: unknown): string => {
  const { text, detail } = plain(error instanceof Error ? error.message : String(error));
  if (detail) {
    originals.delete(text);
    originals.set(text, detail);
    if (originals.size > ORIGINALS_KEPT) originals.delete(originals.keys().next().value!);
  }
  return text;
};

/** The technical text a plain message was made from, when `explain` reworded it. */
export const technicalDetail = (text: string): string | null => originals.get(text) ?? null;
