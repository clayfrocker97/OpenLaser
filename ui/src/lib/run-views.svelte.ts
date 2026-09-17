import { Viewport } from './viewport.svelte';

// Execution state never participates in the key. Returning to a sheet restores
// the same camera, including after the Run component has been unmounted.
const views = new Map<string, { view: Viewport; fitted: boolean }>();

export function runView(key: string): { view: Viewport; fitted: boolean } {
  let current = views.get(key);
  if (!current) {
    current = { view: new Viewport(), fitted: false };
    views.set(key, current);
    if (views.size > 100) views.delete(views.keys().next().value!);
  }
  return current;
}
