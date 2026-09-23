// Press-and-hold confirmation for controls that move the machine, fire the
// beam or set a reference (DESIGN.md, "Touch rules"). A tap does nothing; the
// action fires once, while the press is still down, when the hold completes.
// Unlike lib/hold.ts, releasing after that does not stop anything.

/** How long each kind of control must be held until the server's shared
 *  setting (Settings → Display) has arrived. */
export const HOLD_MS = {
  /** Moves the machine, fires the beam or switches outputs on by itself. */
  move: 1000,
  /** Sets an origin or reference without moving anything. */
  zero: 750,
} as const;

/** The range the server accepts for either hold, so a tap never acts. */
export const MIN_HOLD_MS = 300;
export const MAX_HOLD_MS = 3000;

/** A hold duration for people: 1000 → "1 s", 750 → "0.75 s". */
export const holdSeconds = (ms: number): string => `${Number((ms / 1000).toFixed(2))} s`;

/** The fill only starts after this, so a quick tap does not flash. */
export const HOLD_VISIBLE_AFTER_MS = 150;
/** Moving a press further than this abandons it, as a drag would. */
export const HOLD_SLOP_PX = 12;
/** How long the completed and released-early looks stay. */
export const HOLD_FEEDBACK_MS = 450;

export type HoldPhase = 'idle' | 'holding' | 'done' | 'early';

/** Timers, injectable so tests can drive time. */
export type HoldClock = {
  now: () => number;
  after: (ms: number, run: () => void) => unknown;
  clear: (timer: unknown) => void;
  frame: (run: () => void) => unknown;
};

export const browserClock: HoldClock = {
  now: () => performance.now(),
  after: (ms, run) => setTimeout(run, ms),
  clear: (timer) => clearTimeout(timer as ReturnType<typeof setTimeout>),
  frame: (run) => requestAnimationFrame(run),
};

type Press = { id: number | 'key'; x: number; y: number; start: number; timer: unknown };

/** One control's hold: at most one press at a time. */
export class HoldConfirm {
  phase: HoldPhase = 'idle';
  /** Visible fill, 0 to 1. */
  progress = 0;
  private press: Press | null = null;
  private settle: unknown = null;

  constructor(
    private duration: () => number,
    private fire: () => void,
    private changed: (phase: HoldPhase, progress: number) => void,
    private clock: HoldClock = browserClock,
  ) {}

  /** Starts a press; false when one is already held. */
  begin(id: number | 'key', x = 0, y = 0): boolean {
    if (this.press) return false;
    this.clock.clear(this.settle);
    const start = this.clock.now();
    const press: Press = { id, x, y, start, timer: null };
    press.timer = this.clock.after(this.duration(), () => this.complete(press));
    this.press = press;
    this.show('holding', 0);
    this.clock.frame(() => this.tick(press));
    return true;
  }

  /** A press that wanders or leaves the control is abandoned quietly. */
  move(id: number, x: number, y: number, inside = true): void {
    const press = this.press;
    if (!press || press.id !== id) return;
    if (!inside || Math.hypot(x - press.x, y - press.y) > HOLD_SLOP_PX) this.cancel();
  }

  /** Released before the hold completed: nothing happens, and it says so. */
  end(id: number | 'key'): void {
    const press = this.press;
    if (!press || press.id !== id) return;
    this.stop(press);
    this.show('early', 0);
    this.rest();
  }

  /** Lost, cancelled, blurred or disabled: nothing happens. */
  cancel = (): void => {
    const press = this.press;
    if (!press) return;
    this.stop(press);
    this.show('idle', 0);
  };

  private complete(press: Press): void {
    if (this.press !== press) return;
    this.press = null;
    this.show('done', 1);
    this.rest();
    this.fire();
  }

  private tick(press: Press): void {
    if (this.press !== press) return;
    const held = this.clock.now() - press.start - HOLD_VISIBLE_AFTER_MS;
    const span = Math.max(1, this.duration() - HOLD_VISIBLE_AFTER_MS);
    this.show('holding', Math.min(1, Math.max(0, held / span)));
    this.clock.frame(() => this.tick(press));
  }

  private stop(press: Press): void {
    this.clock.clear(press.timer);
    this.press = null;
  }

  private rest(): void {
    this.clock.clear(this.settle);
    this.settle = this.clock.after(HOLD_FEEDBACK_MS, () => {
      if (!this.press) this.show('idle', 0);
    });
  }

  private show(phase: HoldPhase, progress: number): void {
    this.phase = phase;
    this.progress = progress;
    this.changed(phase, progress);
  }
}
