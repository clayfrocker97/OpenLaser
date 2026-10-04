// What a held finger may still do.

const TEXT = 'input, textarea, [contenteditable]:not([contenteditable="false"])';
/** A control a finger is meant to hold. Marked apart from other buttons so a
 *  long press stays the action. */
const HELD = '.held';

const element = (target: EventTarget | null): Element | null =>
  typeof (target as Element | null)?.closest === 'function' ? target as Element : null;

/** Whether the browser's right-click menu may open on `target`: in text
 *  fields, for paste, and nowhere else. */
export const keepsMenu = (target: EventTarget | null): boolean => !!element(target)?.closest(TEXT);

/** iOS WebKit — Chrome on iPhone is WebKit — does not fire `contextmenu` for
 *  a long press on a button. The copy/paste callout opens instead, and once
 *  it does the page stops receiving touches. `touch-action` and
 *  `-webkit-touch-callout` do not stop it. Cancelling the touch, from a
 *  listener that is not passive, keeps the callout from starting. Other
 *  presses are left alone, so a list can still scroll. */
export function suppressHeldCallout(event: { target: EventTarget | null; cancelable: boolean; preventDefault: () => void }): void {
  if (event.cancelable && element(event.target)?.closest(HELD)) event.preventDefault();
}
