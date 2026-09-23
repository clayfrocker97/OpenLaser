// How many changes wait in Pending changes: unsaved jobs the server retains,
// recipe edits and machine settings staged on this screen. The top bar and
// the phone's Settings show Pending changes only while this is above zero.
import { api } from '../api/client';
import { recipeEdits } from './recipe-edits.svelte';
import { settingsEdits } from './settings-edits.svelte';

/** At most one unsaved-job lookup per this interval, however often the document changes. */
const INTERVAL_MS = 1000;

class Pending {
  /** Unsaved jobs, as the server last reported them. */
  drafts = $state(0);
  private timer: ReturnType<typeof setTimeout> | null = null;
  private serial = 0;

  get count(): number {
    return this.drafts + recipeEdits.pending.length + settingsEdits.pending.length;
  }

  /** Asks the server again shortly; calls while one is scheduled join it. */
  refresh(): void {
    if (this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      const id = ++this.serial;
      api.pendingDrafts().then((drafts) => { if (id === this.serial) this.drafts = drafts.length; }).catch(() => { /* The button stays as it was. */ });
    }, INTERVAL_MS);
  }
}

export const pending = new Pending();
