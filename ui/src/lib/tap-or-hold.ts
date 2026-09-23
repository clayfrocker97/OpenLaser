// A jog key that steps on a tap and jogs while held, as on a machine pendant.
//
// A tap (released before `holdMs`) moves one bounded step when the finger
// lifts. Holding past `holdMs` hands the press to a deadman `Hold`, which
// jogs until release (DESIGN.md rule 5). Sliding off, a cancelled pointer, a
// blurred window or a hidden page do nothing.

/** How long a press must last before it becomes a continuous jog. */
export const CONTINUOUS_AFTER_MS = 400;
/** A finger that travels this far is scrolling or slipping, not tapping. */
const SLOP_PX = 12;

type Pointer = Pick<PointerEvent, 'button' | 'pointerId' | 'clientX' | 'clientY' | 'currentTarget'>;
type Target = Pick<HTMLElement, 'setPointerCapture' | 'hasPointerCapture' | 'releasePointerCapture'>;
/** What a continuous hold needs to take over the press. */
export type Handover = { button: number; pointerId: number; currentTarget: Target };

interface Pending { pointer: number; x: number; y: number; target: Target; timer: ReturnType<typeof setTimeout>; step: () => void }
type Hand = (press: Handover) => void;

export class TapOrHold {
  private pending: Pending | null = null;

  constructor(private available = () => true, private holdMs = CONTINUOUS_AFTER_MS) {}

  /** Starts a press: `step` runs if it ends as a tap, `hold` takes it over if it lasts. */
  press(event: Pointer, step: () => void, hold: Hand): void {
    if (event.button !== 0 || this.pending || !this.available()) return;
    const target = event.currentTarget as unknown as Target;
    try { target.setPointerCapture(event.pointerId); } catch { /* The press still works without capture. */ }
    const pointer = event.pointerId;
    const timer = setTimeout(() => {
      if (this.pending?.pointer !== pointer) return;
      this.pending = null;
      hold({ button: 0, pointerId: pointer, currentTarget: target });
    }, this.holdMs);
    this.pending = { pointer, x: event.clientX, y: event.clientY, target, timer, step };
  }

  /** The finger lifted: a press still pending was a tap. */
  release = (event: Pick<PointerEvent, 'pointerId'>): void => {
    const pending = this.pending;
    if (pending?.pointer !== event.pointerId) return;
    this.clear();
    if (this.available()) pending.step();
  };

  /** The finger moved; beyond the slop the press is abandoned. */
  move = (event: Pick<PointerEvent, 'pointerId' | 'clientX' | 'clientY'>): void => {
    const pending = this.pending;
    if (pending?.pointer === event.pointerId && Math.hypot(event.clientX - pending.x, event.clientY - pending.y) > SLOP_PX) this.cancel();
  };

  /** Abandons a pending press without stepping. */
  cancel = (): void => { this.clear(); };

  private clear(): void {
    const pending = this.pending;
    if (!pending) return;
    this.pending = null;
    clearTimeout(pending.timer);
    try { if (pending.target.hasPointerCapture(pending.pointer)) pending.target.releasePointerCapture(pending.pointer); } catch { /* Detached. */ }
  }

  /** Installed once per component, beside the `Hold` it hands over to. */
  mount(win: Window = window, doc: Document = document): () => void {
    const hidden = () => { if (doc.hidden) this.cancel(); };
    const cancelled = (event: PointerEvent) => { if (this.pending?.pointer === event.pointerId) this.cancel(); };
    win.addEventListener('pointerup', this.release);
    win.addEventListener('pointermove', this.move);
    win.addEventListener('pointercancel', cancelled);
    win.addEventListener('blur', this.cancel);
    doc.addEventListener('visibilitychange', hidden);
    return () => {
      this.cancel();
      win.removeEventListener('pointerup', this.release);
      win.removeEventListener('pointermove', this.move);
      win.removeEventListener('pointercancel', cancelled);
      win.removeEventListener('blur', this.cancel);
      doc.removeEventListener('visibilitychange', hidden);
    };
  }
}
