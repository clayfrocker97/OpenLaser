// The shape most actions share: mark the screen busy, run, report a failure,
// and clear busy whatever happened.
import { ui } from '../stores/ui.svelte';
import { explain } from './format';

/** Reports a failure as an error toast, which stays until dismissed. */
export const sayError = (error: unknown): void => ui.say(explain(error), true);

/**
 * Runs `action` with `setBusy(true)` around it. A failure goes to `onError`
 * (an error toast by default) instead of the caller, and busy is cleared in
 * every case. Callers still check their own busy flag first, where a second
 * press must be refused.
 *
 * ```ts
 * if (busy) return;
 * await withBusy((b) => (busy = b), () => api.saveJob(name));
 * ```
 */
export async function withBusy(
  setBusy: (busy: boolean) => void,
  action: () => Promise<unknown>,
  onError: (error: unknown) => void = sayError,
): Promise<void> {
  setBusy(true);
  try {
    await action();
  } catch (error) {
    onError(error);
  } finally {
    setBusy(false);
  }
}
