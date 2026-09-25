// Drag to reorder tiles: a copy follows the pointer while the real tile
// stays as a placeholder; the order changes only when the pointer crosses
// into another tile, with a pause so the slide animation finishes. Shared
// by Setup's toolbars (ToolBar) and the layer list (JobPanel).

const EASE = 'cubic-bezier(.2,0,0,1)';
/** Travel before a press becomes a drag, so a tap still reaches the tile. */
const START_PX = 6;
/** Pause after a swap while the tiles slide. */
const SETTLE_MS = 200;

interface Options {
  /** The data attribute naming a tile, such as `bar-tool` for `data-bar-tool`. */
  attribute: string;
  /** The current order. */
  order: () => string[];
  /** Saves a new order. */
  save: (order: string[]) => void;
  /** Whether the bar is being edited; outside editing a press is a tap. */
  editing: () => boolean;
  /** Whether the tile's container lays tiles out left to right. */
  horizontal?: (tile: HTMLElement) => boolean;
}

interface Drag { id: string; ghost: HTMLElement; ox: number; oy: number; sx: number; sy: number; x: number; y: number; raf: number; moved: boolean; cool: number }

export class Reorder {
  /** The tile being dragged, shown as a placeholder. */
  dragging = $state<string | null>(null);
  private drag: Drag | null = null;

  constructor(private options: Options) {}

  private get selector(): string { return `[data-${this.options.attribute}]`; }

  down = (e: PointerEvent): void => {
    if (!this.options.editing()) return;
    const el = (e.target as HTMLElement).closest<HTMLElement>(this.selector);
    if (!el) return;
    const r = el.getBoundingClientRect();
    const ghost = el.cloneNode(true) as HTMLElement;
    ghost.classList.remove('editing', 'placeholder');
    Object.assign(ghost.style, { position: 'fixed', left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px`, margin: '0', transform: 'none', pointerEvents: 'none', zIndex: '50' });
    document.body.appendChild(ghost);
    const id = el.dataset[this.datasetKey]!;
    this.drag = { id, ghost, ox: e.clientX - r.left, oy: e.clientY - r.top, sx: e.clientX, sy: e.clientY, x: e.clientX, y: e.clientY, raf: 0, moved: false, cool: 0 };
    el.setPointerCapture(e.pointerId);
    // A tile that moves between the row and the column is a new element,
    // and the old one takes its pointer capture with it; the window still
    // hears the move and the release, so the ghost always goes.
    addEventListener('pointermove', this.move);
    addEventListener('pointerup', this.up);
    addEventListener('pointercancel', this.up);
    e.preventDefault();
  };

  move = (e: PointerEvent): void => {
    const drag = this.drag;
    if (!drag) return;
    drag.x = e.clientX;
    drag.y = e.clientY;
    if (!drag.raf) drag.raf = requestAnimationFrame(this.tick);
  };

  up = (): void => {
    const drag = this.drag;
    if (!drag) return;
    const { id, ghost } = drag;
    this.drag = null;
    removeEventListener('pointermove', this.move);
    removeEventListener('pointerup', this.up);
    removeEventListener('pointercancel', this.up);
    if (drag.raf) cancelAnimationFrame(drag.raf);
    const r = document.querySelector<HTMLElement>(`[data-${this.options.attribute}="${id}"]`)?.getBoundingClientRect();
    const finish = () => { ghost.remove(); this.dragging = null; };
    if (!r) { finish(); return; }
    ghost.animate([{ transform: ghost.style.transform }, { transform: `translate(${r.left - parseFloat(ghost.style.left)}px, ${r.top - parseFloat(ghost.style.top)}px)` }], { duration: 180, easing: EASE }).onfinish = finish;
  };

  /** `data-bar-tool` is read back as `dataset.barTool`. */
  private get datasetKey(): string {
    return this.options.attribute.replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase());
  }

  private tick = (): void => {
    const drag = this.drag;
    if (!drag) return;
    drag.raf = 0;
    if (!drag.moved) {
      if (Math.hypot(drag.x - drag.sx, drag.y - drag.sy) < START_PX) return;
      drag.moved = true;
      this.dragging = drag.id;
    }
    drag.ghost.style.transform = `translate(${drag.x - drag.ox - parseFloat(drag.ghost.style.left)}px, ${drag.y - drag.oy - parseFloat(drag.ghost.style.top)}px)`;
    if (performance.now() < drag.cool) return;
    const hit = document.elementFromPoint(drag.x, drag.y)?.closest<HTMLElement>(this.selector);
    const target = hit?.dataset[this.datasetKey];
    if (!hit || !target || target === drag.id) return;
    const tr = hit.getBoundingClientRect();
    const horizontal = this.options.horizontal?.(hit) ?? true;
    const after = horizontal ? drag.x > tr.left + tr.width / 2 : drag.y > tr.top + tr.height / 2;
    const current = this.options.order();
    const order = current.filter((x) => x !== drag.id);
    order.splice(order.indexOf(target) + (after ? 1 : 0), 0, drag.id);
    if (order.join() === current.join()) return;
    this.options.save(order);
    drag.cool = performance.now() + SETTLE_MS;
    setTimeout(() => { if (this.drag && !this.drag.raf) this.drag.raf = requestAnimationFrame(this.tick); }, SETTLE_MS + 10);
  };
}
