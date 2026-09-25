// The canvas view: which rectangle of the drawing is on screen. Drawing
// coordinates have Y up and the SVG viewBox has Y down, so the view keeps
// SVG units and the stage flips its content once. Zooming out stops at
// the bed with a quarter of it around; zooming in stops at a few
// millimetres across.
import type { Box } from './svg';

/** The room around the bed at the farthest zoom, as a share of its size. */
const MARGIN = 0.25;
/** The narrowest view, in millimetres. */
const CLOSEST = 5;

export class Viewport {
  x = $state(0);
  y = $state(-300);
  w = $state(500);
  h = $state(300);
  /** Width over height of the element, and its width in pixels; the stage keeps both current. */
  private aspect = 5 / 3;
  private pixels = $state(1000);
  private measured = false;
  /** The bed: the farthest the view goes. */
  private limit: Box | null = null;
  /** What was last fitted, applied again when the element's aspect becomes known. */
  private request: { box: Box; margin: number } | null = null;
  private fittedBox: { box: Box; margin: number } | null = null;
  /** The view the drawing is rendered at. It follows the view, except while
   *  a gesture holds it: the stage then shows the view by moving the picture
   *  already drawn, and the drawing is rendered again only when it settles. */
  shown = $state.raw({ x: 0, y: -300, w: 500, h: 300 });
  private holding = false;

  get viewBox(): string {
    const { x, y, w, h } = this.shown;
    return `${x} ${y} ${w} ${h}`;
  }

  /** Drawing millimetres per screen pixel, as the drawing is rendered. */
  get shownMmPerPixel(): number {
    return this.shown.w / this.pixels;
  }

  /** Keeps the rendered view while a gesture moves the view. */
  hold(): void {
    this.holding = true;
  }

  /** Renders the view as it is now, held or not. */
  settle(): void {
    const { x, y, w, h } = this;
    const s = this.shown;
    if (s.x !== x || s.y !== y || s.w !== w || s.h !== h) this.shown = { x, y, w, h };
  }

  /** Ends a hold, rendering the view as it is now. */
  release(): void {
    this.holding = false;
    this.settle();
  }

  private changed(): void {
    if (!this.holding) this.settle();
  }

  /** Zoom relative to the whole bed, which is 100%. */
  get percent(): number {
    return Math.round((this.farthest() / this.w) * 100);
  }

  /** Drawing millimetres per screen pixel. */
  get mmPerPixel(): number {
    return this.w / this.pixels;
  }

  /** The element's width in pixels. */
  get pixelWidth(): number {
    return this.pixels;
  }

  /** The bed, which bounds how far the view zooms out. */
  setLimit(bed: Box | null): void {
    this.limit = bed;
  }

  /** Shows the whole bed with room around it. */
  home(): void {
    if (this.limit) this.fit(this.limit, MARGIN);
  }

  /** Shows `box` with a margin around it. */
  fit(box: Box, margin = MARGIN): void {
    this.fittedBox = { box, margin };
    this.request = this.measured ? null : { box, margin };
    const w = this.clamped(this.widthFor(box, margin));
    const h = w / this.aspect;
    this.x = (box.minX + box.maxX) / 2 - w / 2;
    this.y = -(box.minY + box.maxY) / 2 - h / 2;
    this.w = w;
    this.h = h;
    this.changed();
  }

  /** The element measures `width` by `height` pixels. */
  resize(width: number, height: number): void {
    if (!(width > 0 && height > 0)) return;
    const scale = this.mmPerPixel;
    const cx = this.x + this.w / 2, cy = this.y + this.h / 2;
    const first = !this.measured;
    const request = this.request;
    this.measured = true;
    this.pixels = width;
    const aspect = width / height;
    this.aspect = aspect;
    if (first && request) {
      this.fit(request.box, request.margin);
    } else {
      this.w = scale * width;
      this.h = scale * height;
      this.x = cx - this.w / 2;
      this.y = cy - this.h / 2;
    }
    this.changed();
  }

  /** Zooms by `factor` about an SVG point, the centre by default. */
  zoom(factor: number, at?: [number, number]): void {
    const [px, py] = at ?? [this.x + this.w / 2, this.y + this.h / 2];
    const w = this.clamped(this.w / factor);
    const f = this.w / w;
    this.request = null;
    this.x = px - (px - this.x) / f;
    this.y = py - (py - this.y) / f;
    this.w = w;
    this.h = w / this.aspect;
    this.changed();
  }

  /** Shows the view `w` wide with the SVG point `anchor` under the screen point `client`. */
  place(w: number, anchor: [number, number], client: [number, number], rect: DOMRect): void {
    const width = this.clamped(w);
    const height = width / this.aspect;
    this.request = null;
    this.x = anchor[0] - ((client[0] - rect.left) * width) / rect.width;
    this.y = anchor[1] - ((client[1] - rect.top) * height) / rect.height;
    this.w = width;
    this.h = height;
    this.changed();
  }

  /** Moves the view by SVG units. */
  pan(dx: number, dy: number): void {
    this.request = null;
    this.x += dx;
    this.y += dy;
    this.changed();
  }

  /** The view width that shows `box` with `margin` around it at this aspect. */
  private widthFor(box: Box, margin: number): number {
    const w = Math.max(box.maxX - box.minX, 1) * (1 + 2 * margin);
    const h = Math.max(box.maxY - box.minY, 1) * (1 + 2 * margin);
    return Math.max(w, h * this.aspect);
  }

  /** The widest view: the bed with a quarter of it around, or eight times the last fit. */
  private farthest(): number {
    return this.limit ? this.widthFor(this.limit, MARGIN) : (this.fittedBox ? this.widthFor(this.fittedBox.box, this.fittedBox.margin) : this.w) * 8;
  }

  private clamped(w: number): number {
    return Math.min(Math.max(w, CLOSEST), this.farthest());
  }
}
