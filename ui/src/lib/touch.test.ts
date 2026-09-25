import { describe, expect, it } from 'vitest';
import { keepsMenu } from './touch';

/** An element inside a text field, or not. */
const element = (inText: boolean) => ({ closest: (selector: string) => (inText && selector.includes('input') ? {} : null) }) as unknown as EventTarget;

describe('held finger', () => {
  it('keeps the menu only in text fields', () => {
    expect(keepsMenu(element(false))).toBe(false);
    expect(keepsMenu(element(true))).toBe(true);
    expect(keepsMenu(null)).toBe(false);
    expect(keepsMenu({} as EventTarget)).toBe(false);
  });
});
