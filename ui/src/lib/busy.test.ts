import { describe, expect, it, vi } from 'vitest';
const { say } = vi.hoisted(() => ({ say: vi.fn() }));
vi.mock('../stores/ui.svelte', () => ({ ui: { say } }));
import { withBusy } from './busy';

describe('withBusy', () => {
  it('is busy only while the action runs', async () => {
    const states: boolean[] = [];
    let during: boolean | undefined;
    await withBusy((b) => states.push(b), async () => { during = states.at(-1); });
    expect(during).toBe(true);
    expect(states).toEqual([true, false]);
  });

  it('reports a failure as an error toast by default and clears busy', async () => {
    say.mockClear();
    const states: boolean[] = [];
    await withBusy((b) => states.push(b), () => Promise.reject(new Error('No machine')));
    expect(states).toEqual([true, false]);
    expect(say).toHaveBeenCalledWith(expect.stringContaining('No machine'), true);
  });

  it('hands a failure to a given handler instead', async () => {
    say.mockClear();
    const seen: unknown[] = [];
    await withBusy(() => undefined, () => Promise.reject(new Error('bad')), (e) => seen.push(e));
    expect(seen).toHaveLength(1);
    expect(say).not.toHaveBeenCalled();
  });
});
