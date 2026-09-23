import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HOLD_FEEDBACK_MS, HOLD_MS, HOLD_SLOP_PX, HOLD_VISIBLE_AFTER_MS, HoldConfirm, type HoldClock } from './hold-confirm';

// Fake timers drive both the completion timer and the animation frames.
const clock: HoldClock = {
  now: () => Date.now(),
  after: (ms, run) => setTimeout(run, ms),
  clear: (timer) => clearTimeout(timer as ReturnType<typeof setTimeout>),
  frame: (run) => setTimeout(run, 16),
};

function setup(duration: number = HOLD_MS.move) {
  const fire = vi.fn();
  const seen: string[] = [];
  const hold = new HoldConfirm(() => duration, fire, (phase) => { if (seen.at(-1) !== phase) seen.push(phase); }, clock);
  return { hold, fire, seen };
}

describe('hold to confirm', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('fires once at the threshold while the press is still down', () => {
    const { hold, fire, seen } = setup();
    expect(hold.begin(1)).toBe(true);
    vi.advanceTimersByTime(HOLD_MS.move - 1);
    expect(fire).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fire).toHaveBeenCalledTimes(1);
    hold.end(1);
    vi.advanceTimersByTime(HOLD_MS.move * 2);
    expect(fire).toHaveBeenCalledTimes(1);
    expect(seen).toEqual(['holding', 'done', 'idle']);
  });

  it('does nothing for a tap and says it needs holding', () => {
    const { hold, fire, seen } = setup();
    hold.begin(1);
    vi.advanceTimersByTime(200);
    hold.end(1);
    expect(hold.phase).toBe('early');
    vi.advanceTimersByTime(HOLD_FEEDBACK_MS);
    expect(hold.phase).toBe('idle');
    vi.advanceTimersByTime(HOLD_MS.move);
    expect(fire).not.toHaveBeenCalled();
    expect(seen).toEqual(['holding', 'early', 'idle']);
  });

  it('shows no fill for a quick tap and a full fill at the threshold', () => {
    const { hold } = setup();
    hold.begin(1);
    vi.advanceTimersByTime(HOLD_VISIBLE_AFTER_MS - 20);
    expect(hold.progress).toBe(0);
    const middle = HOLD_VISIBLE_AFTER_MS + (HOLD_MS.move - HOLD_VISIBLE_AFTER_MS) / 2;
    vi.advanceTimersByTime(middle - (HOLD_VISIBLE_AFTER_MS - 20));
    expect(hold.progress).toBeGreaterThan(0.4);
    expect(hold.progress).toBeLessThan(0.6);
    vi.advanceTimersByTime(HOLD_MS.move - middle);
    expect([hold.phase, hold.progress]).toEqual(['done', 1]);
    vi.advanceTimersByTime(HOLD_FEEDBACK_MS);
    expect([hold.phase, hold.progress]).toEqual(['idle', 0]);
  });

  it('abandons a press that drags, leaves or is cancelled, and ignores other pointers', () => {
    for (const abandon of [
      (hold: HoldConfirm) => hold.move(1, HOLD_SLOP_PX + 1, 0),
      (hold: HoldConfirm) => hold.move(1, 1, 1, false),
      (hold: HoldConfirm) => hold.cancel(),
    ]) {
      const { hold, fire } = setup();
      hold.begin(1, 0, 0);
      hold.move(1, HOLD_SLOP_PX - 1, 0);
      hold.move(2, 500, 500, false);
      hold.end(2);
      expect(hold.phase).toBe('holding');
      abandon(hold);
      expect(hold.phase).toBe('idle');
      vi.advanceTimersByTime(HOLD_MS.move * 2);
      expect(fire).not.toHaveBeenCalled();
    }
  });

  it('holds one press at a time and a keyboard hold counts like a pointer', () => {
    const { hold, fire } = setup(HOLD_MS.zero);
    expect(hold.begin('key')).toBe(true);
    expect(hold.begin(1)).toBe(false);
    vi.advanceTimersByTime(HOLD_MS.zero);
    expect(fire).toHaveBeenCalledTimes(1);
    expect(hold.begin(1)).toBe(true);
    hold.end(1);
    vi.advanceTimersByTime(HOLD_MS.zero);
    expect(fire).toHaveBeenCalledTimes(1);
  });
});
