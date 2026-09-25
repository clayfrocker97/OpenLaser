import { describe, expect, it } from 'vitest';
import { Viewport } from './viewport.svelte';
import { runView } from './run-views.svelte';

const bed = { minX: 0, minY: 0, maxX: 1371, maxY: 950 };
const center = (view: Viewport) => [view.x + view.w / 2, view.y + view.h / 2];

describe('an operator-owned Run viewport', () => {
  it('keeps the inspected cut and physical zoom when execution controls resize the canvas', () => {
    const view = new Viewport();
    view.setLimit(bed); view.home(); view.resize(720, 600);
    view.zoom(9); view.pan(37, -21);
    const position = center(view), scale = view.mmPerPixel;
    view.resize(720, 690);
    view.resize(660, 550);
    expect(view.mmPerPixel).toBeCloseTo(scale, 12);
    center(view).forEach((v, i) => expect(v).toBeCloseTo(position[i]!, 10));
  });

  it('fits once, then preserves the view even when the operator has not zoomed yet', () => {
    const view = new Viewport();
    view.setLimit(bed); view.home(); view.resize(800, 500);
    const scale = view.mmPerPixel, position = center(view);
    view.resize(800, 650);
    expect(view.mmPerPixel).toBe(scale);
    expect(center(view)).toEqual(position);
  });

  it('restores each sheet after leaving Run without reinitializing its camera', () => {
    const first = runView('viewport-test/sheet-1');
    first.view.setLimit(bed); first.view.home(); first.view.resize(800, 600);
    first.view.zoom(7); first.view.pan(20, -35); first.fitted = true;
    const before = first.view.viewBox;
    const second = runView('viewport-test/sheet-2');
    second.view.fit({ minX: 10, minY: 10, maxX: 50, maxY: 70 });
    expect(runView('viewport-test/sheet-1').view.viewBox).toBe(before);
    expect(runView('viewport-test/sheet-1').fitted).toBe(true);
    expect(second.view).not.toBe(first.view);
  });

  it('keeps the drawn view through a held gesture and draws the view on release', () => {
    const view = new Viewport();
    view.setLimit(bed); view.home(); view.resize(800, 500);
    const drawn = view.viewBox;
    view.hold();
    view.zoom(2); view.pan(10, 5);
    expect(view.viewBox).toBe(drawn);
    expect(view.shownMmPerPixel).toBeCloseTo(view.mmPerPixel * 2, 12);
    view.release();
    expect(view.viewBox).toBe(`${view.x} ${view.y} ${view.w} ${view.h}`);
    view.zoom(2);
    expect(view.shown.w).toBe(view.w);
  });
});
