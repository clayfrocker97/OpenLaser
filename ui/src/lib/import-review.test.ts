import { describe, expect, it } from 'vitest';
import type { ImportReview } from '../api';
import { contourKinds, layersChoosable, needsReview, repairLines, scaleChoosable, toggleLayer } from './import-review';

const clean = (): ImportReview => ({
  name: 'plate.dxf',
  layers: [{ name: '0', hidden: false, imported: true, entities: 4 }],
  scale: null,
  repairs: { gaps: [], duplicates: [], mirrored: [] },
  size: { size: [100, 50], bed: [1500, 3000], exceeds_bed: false, tiny: false },
  bounds: null,
  contours: 2,
  open: [],
  crossing: [],
  outline: [[[0, 0], [1, 0], [1, 1], [0, 0]], [[2, 2], [3, 3]]],
  warnings: [],
});

describe('import review', () => {
  it('imports a clean file straight away and reviews one with anything to check', () => {
    expect(needsReview(clean())).toBe(false);
    const cases: ((r: ImportReview) => void)[] = [
      (r) => { r.scale = { units: 'ambiguous', scale: 'dpi96' }; },
      (r) => { r.layers.push({ name: 'Notes', hidden: true, imported: false, entities: 3 }); },
      (r) => { r.repairs.gaps.push({ at: { x: 1, y: 2 }, size: 0.02, layer: '0' }); },
      (r) => { r.size!.tiny = true; },
      (r) => { r.size!.exceeds_bed = true; },
      (r) => { r.open = [1]; },
      (r) => { r.crossing = [0]; },
      (r) => { r.contours = 0; },
    ];
    for (const change of cases) {
      const review = clean();
      change(review);
      expect(needsReview(review)).toBe(true);
    }
  });

  it('offers scale and layer choices only where they apply', () => {
    const review = clean();
    expect(scaleChoosable(review)).toBe(false);
    review.scale = { units: 'physical', scale: null };
    expect(scaleChoosable(review)).toBe(false);
    review.scale = { units: 'illustrator', scale: 'dpi72' };
    expect(scaleChoosable(review)).toBe(true);
    expect(layersChoosable(review)).toBe(false);
    review.layers.push({ name: 'Etch', hidden: false, imported: true, entities: 1 });
    expect(layersChoosable(review)).toBe(true);
  });

  it('toggles layers from what is imported now', () => {
    const layers = [
      { name: 'Cut', hidden: false, imported: true, entities: 1 },
      { name: 'Notes', hidden: true, imported: false, entities: 1 },
      { name: 'Etch', hidden: false, imported: true, entities: 1 },
    ];
    expect(toggleLayer(layers, 'Notes', true)).toEqual(['Cut', 'Notes', 'Etch']);
    expect(toggleLayer(layers, 'Cut', false)).toEqual(['Etch']);
  });

  it('summarises repairs and marks each contour', () => {
    const review = clean();
    review.repairs.gaps.push({ at: { x: 0, y: 0 }, size: 0.01, layer: '0' }, { at: { x: 0, y: 0 }, size: 0.04, layer: '0' });
    review.repairs.mirrored.push({ at: { x: 0, y: 0 }, size: 0, layer: '0' });
    expect(repairLines(review.repairs)).toEqual(['2 gaps closed, the widest 0.040 mm', '1 upside-down entity mirrored into place']);
    review.open = [1];
    review.crossing = [0];
    expect(contourKinds(review)).toEqual(['crossing', 'open']);
  });
});
