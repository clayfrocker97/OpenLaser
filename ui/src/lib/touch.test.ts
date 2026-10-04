import { describe, expect, it } from 'vitest';
import { keepsMenu, suppressHeldCallout } from './touch';

/** An element `closest` can find inside a text field, a held control, both, or neither. */
const element = (where: { text?: boolean; held?: boolean } = {}) => ({
  closest: (selector: string) => ((where.text && selector.includes('input')) || (where.held && selector.includes('held')) ? {} : null),
}) as unknown as EventTarget;

describe('held finger', () => {
  it('keeps the menu only in text fields', () => {
    expect(keepsMenu(element())).toBe(false);
    expect(keepsMenu(element({ text: true }))).toBe(true);
    expect(keepsMenu(null)).toBe(false);
    expect(keepsMenu({} as EventTarget)).toBe(false);
  });

  it('cancels a touch on a held control and leaves every other touch alone', () => {
    const cancelled = (target: EventTarget | null, cancelable = true) => {
      let prevented = false;
      suppressHeldCallout({ target, cancelable, preventDefault: () => { prevented = true; } });
      return prevented;
    };
    expect(cancelled(element({ held: true }))).toBe(true);
    expect(cancelled(element({ held: true, text: true }))).toBe(true);
    expect(cancelled(element({ held: true }), false)).toBe(false);
    expect(cancelled(element())).toBe(false);
    expect(cancelled(element({ text: true }))).toBe(false);
    expect(cancelled(null)).toBe(false);
    expect(cancelled({} as EventTarget)).toBe(false);
  });
});
