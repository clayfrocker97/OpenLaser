import { describe, expect, it } from 'vitest';
import { layerColor, scoped, setOwn } from './drawing-layers';
import type { DraftLayer, Features } from '../api';

const layer = (name: string, color: [number, number, number] | null = null): DraftLayer =>
  ({ name, contours: 1, output: true, chosen: true, recipe: null, mode: 'cut', color, machining: null });
const job: Features = {
  leads: null, joints: null, cooling: null, kerf: { width: 0.2, side: 'auto' }, bridges: null, common: null,
  start: { position: 'keep', direction: 'keep', spots: [{ contour: 2, fraction: 0.5 }] }, seam: 'seal',
  order: { strategy: 'as_drawn', inner_first: true, circles_first: false, spread_heat: false }, skip_layers: [],
};

describe('layers', () => {
  it('draws the file colour, else the cut colour first and the palette after', () => {
    const layers = [layer('0'), layer('Red', [255, 0, 0]), layer('Etch'), layer('White', [255, 255, 255]), layer('Blue', [0, 0, 255])];
    expect(layerColor(layers, '0')).toBeNull();
    expect(layerColor(layers, 'Red')).toBe('#ff0000');
    expect(layerColor(layers, 'Etch')).toBe('#3b82f6');
    expect(layerColor(layers, 'White')).toBeNull();
    expect(layerColor(layers, 'Blue')).toBe('#0000ff');
    // Another order keeps every colour.
    const turned = [...layers].reverse();
    expect(layers.map((l) => layerColor(turned, l.name))).toEqual(layers.map((l) => layerColor(layers, l.name)));
  });

  it('sees a layer through its own machining, keeping the jobs picked places', () => {
    expect(scoped(job, 'CUT')).toBe(job);
    const marked: Features = { ...job, layers: [{ name: 'Mark', mode: 'mark', color: null, machining: null }] };
    expect(scoped(marked, 'Mark').kerf).toBeNull();
    expect(scoped(marked, 'Mark').start.spots).toEqual(job.start.spots);
    const edited = structuredClone(marked);
    setOwn(edited, 'Cut', { ...job, kerf: null }, ['Mark', 'Cut']);
    expect(edited.layers?.map((l) => l.name)).toEqual(['Mark', 'Cut']);
    expect(scoped(edited, 'Cut').kerf).toBeNull();
    expect(edited.layers?.[1]?.machining?.start.spots).toEqual([]);
  });
});
