import { shown } from './recipe';
import { quantity } from './units.svelte';

/** A read-only view of typed saved values; never passed back to an API. */
export function reviewValue(value: unknown, path = ''): unknown {
  const key = path.split('/').at(-1) ?? '';
  if (typeof value === 'string' && /\/(recipe|film)\/attributes\/[^/]+$/.test(path)) return shown(key, value);
  if (typeof value === 'number') {
    if (key === 'thickness_mm') return quantity(value, 'mm');
    if (/^\/features\//.test(path)) {
      if (['length', 'radius', 'width', 'minimum_size', 'tolerance', 'spacing', 'overcut', 'gap'].includes(key) || /\/point\/[xy]$/.test(path)) return quantity(value, 'mm');
      if (key === 'slow_speed') return quantity(value, 'mm/s');
    }
    if (/^\/nesting\//.test(path) && ['width', 'height', 'spacing', 'margin', 'remnant_clearance', 'clearance', 'x', 'y'].includes(key)) return quantity(value, 'mm');
    if (/\/action\/[xy]$/.test(path)) return quantity(value, 'mm');
    if (/\/action\/pressure$/.test(path)) return quantity(value, 'bar');
  }
  if (Array.isArray(value)) {
    if (path === '/zero' || path === '/placement/origin') return value.map(n => quantity(n, 'mm'));
    if (/^\/placed\/\d+\/transform$/.test(path)) return value.map((n, index) => index >= 4 ? quantity(n, 'mm') : n);
    return value.map((item, index) => reviewValue(item, `${path}/${index}`));
  }
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, reviewValue(item, `${path}/${key}`)]));
  return value;
}

export function reviewText(value: unknown, path = ''): string {
  const display = reviewValue(value, path);
  return typeof display === 'string' ? display : JSON.stringify(display);
}

export function conflictText(value: string, path: string): string {
  try { return reviewText(JSON.parse(value), path); } catch { return value; }
}
