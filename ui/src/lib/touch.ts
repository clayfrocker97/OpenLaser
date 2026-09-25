// What a held finger may still do.

const TEXT = 'input, textarea, [contenteditable]:not([contenteditable="false"])';

/** Whether the browser's right-click menu may open on `target`: in text
 *  fields, for paste, and nowhere else. */
export const keepsMenu = (target: EventTarget | null): boolean =>
  typeof (target as Element | null)?.closest === 'function' && !!(target as Element).closest(TEXT);
