import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Hold } from './hold';
import type { Lease } from '../api';

const deferred = () => {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

function setup() {
  const transport = { heartbeat: vi.fn<(lease: Lease) => Promise<unknown>>().mockResolvedValue(null), release: vi.fn<(lease: Lease) => Promise<unknown>>().mockResolvedValue(null) };
  const error = vi.fn();
  const hold = new Hold(transport, error);
  const target = { setPointerCapture: vi.fn(), hasPointerCapture: () => true, releasePointerCapture: vi.fn() };
  const press = (pointerId = 1) => ({ button: 0, pointerId, currentTarget: target as unknown as HTMLElement });
  return { hold, transport, error, press, target };
}

describe('owned held controls', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('releases before admission and an old reply cannot renew a newer press', async () => {
    const { hold, transport, press } = setup();
    const first = deferred();
    const start = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(null);
    hold.press(press(), start);
    hold.release({ pointerId: 1 });
    hold.press(press(2), start);
    first.resolve();
    await vi.advanceTimersByTimeAsync(160);
    expect(transport.release).toHaveBeenCalledWith(start.mock.calls[0]![0]);
    expect(transport.heartbeat.mock.calls.every(([lease]) => lease.sequence === start.mock.calls[1]![0].sequence)).toBe(true);
    hold.cancel();
    const count = transport.heartbeat.mock.calls.length;
    await vi.advanceTimersByTimeAsync(500);
    expect(transport.heartbeat).toHaveBeenCalledTimes(count);
  });

  it('never overlaps renewal requests and ignores a second pointer', async () => {
    const { hold, transport, press } = setup();
    const renewal = deferred();
    transport.heartbeat.mockReturnValue(renewal.promise);
    const start = vi.fn().mockResolvedValue(null);
    hold.press(press(), start);
    hold.press(press(2), start);
    hold.release({ pointerId: 2 });
    await vi.advanceTimersByTimeAsync(500);
    expect(start).toHaveBeenCalledTimes(1);
    expect(transport.heartbeat).toHaveBeenCalledTimes(1);
    hold.cancel();
    renewal.resolve();
    await vi.advanceTimersByTimeAsync(500);
    expect(transport.heartbeat).toHaveBeenCalledTimes(1);
  });

  it('a refused start releases without starting a timer', async () => {
    const { hold, transport, error, press } = setup();
    hold.press(press(), () => Promise.reject(new Error('busy')));
    await vi.advanceTimersByTimeAsync(500);
    expect(error).toHaveBeenCalledOnce();
    expect(transport.release).toHaveBeenCalledOnce();
    expect(transport.heartbeat).not.toHaveBeenCalled();
  });

  it.each(['pointerup', 'pointercancel', 'lostpointercapture', 'blur', 'visibilitychange', 'unmount'])('terminates on %s outside the button', async (event) => {
    const { hold, transport, press } = setup();
    const win = new EventTarget();
    const doc = Object.assign(new EventTarget(), { hidden: true });
    const unmount = hold.mount(win as Window, doc as unknown as Document);
    hold.press(press(), () => Promise.resolve());
    if (event === 'unmount') unmount();
    else if (event === 'visibilitychange') doc.dispatchEvent(new Event(event));
    else win.dispatchEvent(Object.assign(new Event(event), { pointerId: 1 }));
    await vi.advanceTimersByTimeAsync(500);
    expect(transport.release).toHaveBeenCalledOnce();
    expect(transport.heartbeat).not.toHaveBeenCalled();
    unmount();
  });

  it('a detached capture target cannot prevent release', async () => {
    const { hold, transport, press, target } = setup();
    target.releasePointerCapture.mockImplementation(() => { throw new Error('detached'); });
    hold.press(press(), () => Promise.resolve());
    hold.cancel();
    await vi.advanceTimersByTimeAsync(500);
    expect(transport.release).toHaveBeenCalledOnce();
    expect(transport.heartbeat).not.toHaveBeenCalled();
  });
});
