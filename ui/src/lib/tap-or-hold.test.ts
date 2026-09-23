import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TapOrHold, type Handover } from './tap-or-hold';

const target = () => ({ setPointerCapture: vi.fn(), hasPointerCapture: vi.fn(() => true), releasePointerCapture: vi.fn() });
const down = (pointerId = 1, clientX = 0, clientY = 0) => ({ button: 0, pointerId, clientX, clientY, currentTarget: target() as unknown as EventTarget });

describe('TapOrHold', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('steps once on a tap and never holds', () => {
    const hold = vi.fn(), step = vi.fn();
    const key = new TapOrHold();
    key.press(down(), step, hold);
    vi.advanceTimersByTime(200);
    key.release({ pointerId: 1 });
    vi.advanceTimersByTime(1000);
    expect(step).toHaveBeenCalledTimes(1);
    expect(hold).not.toHaveBeenCalled();
  });

  it('hands a long press to the deadman hold and does not step', () => {
    const handovers: Handover[] = [];
    const step = vi.fn();
    const key = new TapOrHold();
    key.press(down(7), step, (press) => handovers.push(press));
    vi.advanceTimersByTime(400);
    key.release({ pointerId: 7 });
    expect(handovers).toHaveLength(1);
    expect(handovers[0]!.pointerId).toBe(7);
    expect(step).not.toHaveBeenCalled();
  });

  it('abandons a press that slides away or is cancelled', () => {
    const hold = vi.fn(), step = vi.fn();
    const key = new TapOrHold();
    key.press(down(1, 0, 0), step, hold);
    key.move({ pointerId: 1, clientX: 20, clientY: 0 });
    key.release({ pointerId: 1 });
    key.press(down(2), step, hold);
    key.cancel();
    vi.advanceTimersByTime(1000);
    key.release({ pointerId: 2 });
    expect(step).not.toHaveBeenCalled();
    expect(hold).not.toHaveBeenCalled();
  });

  it('ignores presses while unavailable and other pointers', () => {
    const step = vi.fn();
    let available = false;
    const key = new TapOrHold(() => available);
    key.press(down(), step, vi.fn());
    key.release({ pointerId: 1 });
    available = true;
    key.press(down(1), step, vi.fn());
    key.release({ pointerId: 2 });
    expect(step).not.toHaveBeenCalled();
    key.release({ pointerId: 1 });
    expect(step).toHaveBeenCalledTimes(1);
  });
});
