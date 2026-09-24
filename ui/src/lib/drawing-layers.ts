// The drawing's layers on screen: the colour each draws in, and machining seen
// from one layer. A layer's machining of its own lives in the features'
// layer table, so editing it is a features edit like any other.
import type { DraftLayer, Features, Layer, Machining } from '../api';

/** Colours for layers the file leaves uncoloured, after the first, which
 *  draws in the screen's cut colour; distinct on light and dark. */
export const PALETTE = ['#f28c28', '#3b82f6', '#d946ef', '#eab308', '#06b6d4', '#ef4444', '#8b5cf6', '#84cc16', '#ec4899', '#14b8a6'];

const hex = ([r, g, b]: [number, number, number]) => `#${[r, g, b].map((c) => c.toString(16).padStart(2, '0')).join('')}`;

/** The colour a layer draws in; null draws in the screen's cut colour.
 *  White and black from a file read as the screen's own ink, which a light
 *  or dark screen would otherwise lose. */
export function layerColor(layers: readonly DraftLayer[], name: string): string | null {
  const at = layers.findIndex((l) => l.name === name);
  const layer = layers[at];
  if (!layer) return null;
  if (layer.color) {
    const grey = Math.max(...layer.color) - Math.min(...layer.color) < 24;
    const level = Math.max(...layer.color);
    return grey && (level > 230 || level < 40) ? null : hex(layer.color);
  }
  const uncoloured = layers.slice(0, at).filter((l) => !l.color).length;
  return uncoloured === 0 ? null : PALETTE[(uncoloured - 1) % PALETTE.length]!;
}

/** The machining features a layer can have its own of. */
export const LAYER_MACHINING = ['leads', 'joints', 'cooling', 'kerf', 'start'] as const;

const BARE: Machining = { leads: null, joints: null, cooling: null, kerf: null, start: { position: 'keep', direction: 'keep', spots: [] }, seam: 'seal' };

/** The features as one layer's contours see them: its own machining over the
 *  job's, keeping the job's picked places; a mark without its own has none. */
export function scoped(features: Features, layer: string | null): Features {
  const entry = layer ? features.layers?.find((l) => l.name === layer) : undefined;
  const own = entry?.machining ?? (entry?.mode === 'mark' ? BARE : null);
  if (!own) return features;
  return {
    ...features,
    leads: own.leads ? { ...own.leads, overrides: features.leads?.overrides ?? [] } : null,
    joints: own.joints, cooling: own.cooling, kerf: own.kerf,
    start: { ...own.start, spots: features.start.spots }, seam: own.seam,
  };
}

/** Whether the layer has machining of its own. */
export const hasOwn = (features: Features, layer: string): boolean =>
  !!features.layers?.find((l) => l.name === layer)?.machining;

/** The layer table with every layer, in `order`, so adding one entry never
 *  moves a layer ahead of the others. */
export function table(features: Features, order: readonly string[]): Layer[] {
  return order.map((name) => features.layers?.find((l) => l.name === name) ?? { name, mode: 'cut', color: null, machining: null });
}

/** Writes `view`'s machining to the layer as its own; none gives it back to the job. */
export function setOwn(features: Features, layer: string, view: Features | null, order: readonly string[]): void {
  const layers = table(features, order);
  const entry = layers.find((l) => l.name === layer);
  if (!entry) return;
  entry.machining = view && {
    leads: view.leads ? { ...view.leads, overrides: [] } : null,
    joints: view.joints, cooling: view.cooling, kerf: view.kerf,
    start: { ...view.start, spots: [] }, seam: view.seam,
  };
  features.layers = layers;
}
