// What every feature form shares: the settings being edited and how an edit
// reaches the server, which the panel owns.
import type { Features, Spot } from '../../../api';
import { osk } from '../../../lib/osk.svelte';
import { ui } from '../../../stores/ui.svelte';

/** Stages an edit of the job's features. */
export type SetFeatures = (change: (f: Features) => void) => void;

/** A feature whose places are picked on the drawing. */
export type PickFeature = 'joints' | 'cooling' | 'start' | 'bridges' | 'order';

/** The picked places of a placement, none for any other kind. */
export const spots = (placement: { manual: Spot[] } | object): Spot[] => ('manual' in placement ? placement.manual : []);

/** Asks for positions along every contour, as percentages, and places the
 *  feature at each on all `contours` of the drawing. */
export function askPercentages(features: Features, feature: 'joints' | 'cooling', contours: number, set: SetFeatures): void {
  const placement = features[feature]?.placement;
  const current = placement && 'manual' in placement
    ? [...new Set(placement.manual.map(s => +(s.fraction * 100).toFixed(3)))].join(', ')
    : '';
  osk.text('Positions on each contour (%)', current, (text) => {
    const values = text.trim() ? text.split(',').map(v => v.trim() ? Number(v.trim()) : NaN) : [];
    if (values.some(v => !Number.isFinite(v) || v < 0 || v > 100)) {
      ui.say('Enter comma-separated percentages from 0 to 100.', true);
      return;
    }
    const unique = [...new Set(values)];
    const manual = Array.from({ length: contours }, (_, contour) => unique.map(v => ({ contour, fraction: v / 100 }))).flat();
    set(f => { const target = f[feature]; if (target) target.placement = { manual }; });
  });
}
