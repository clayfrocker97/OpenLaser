import type { DraftView, Features } from '../api';

type Edit = { key: string; generation: number; revision: number; features: Features; version: number; failed: string | null };

/** One optimistic feature snapshot, flushed serially against acknowledged revisions. */
export class FeatureEdits {
  private edit = $state<Edit | null>(null);
  private flight: Promise<void> | null = null;

  constructor(private current: () => DraftView | null, private save: (features: Features, revision: number) => Promise<DraftView>) {}

  private owns(draft: DraftView | null): boolean {
    return draft !== null && this.edit?.key === draft.key && this.edit.generation === draft.generation;
  }
  value(draft: DraftView): Features { return this.owns(draft) ? this.edit!.features : draft.features; }
  get error(): string | null { return this.owns(this.current()) ? this.edit?.failed ?? null : null; }
  get busy(): boolean { return this.owns(this.current()) && this.flight !== null; }
  discard(): void { this.edit = null; }

  change(change: (features: Features) => void): Promise<void> {
    const draft = this.current();
    if (!draft) return Promise.reject(new Error('Open a part first.'));
    if (this.flight && !this.owns(draft)) return Promise.reject(new Error('The previous feature edit is still finishing.'));
    if (!this.edit || !this.owns(draft)) {
      this.edit = { key: draft.key, generation: draft.generation, revision: draft.revision, features: structuredClone($state.snapshot(draft.features)), version: 0, failed: null };
    }
    const edit = this.edit;
    if (edit.failed) return Promise.reject(new Error('Discard the refused edits before editing again.'));
    const features = structuredClone($state.snapshot(edit.features));
    change(features);
    edit.features = features;
    edit.version += 1;
    if (!this.flight) {
      this.flight = this.flush(edit).finally(() => { this.flight = null; });
    }
    return this.flight;
  }

  private async flush(edit: Edit): Promise<void> {
    try {
      while (this.edit === edit) {
        const current = this.current();
        if (current?.key !== edit.key || current.generation !== edit.generation || current.revision > edit.revision) throw new Error('The draft changed before the edits could be saved.');
        const version = edit.version;
        const saved = await this.save(structuredClone($state.snapshot(edit.features)), edit.revision);
        if (this.edit !== edit) return;
        if (saved.key !== edit.key || saved.generation !== edit.generation) throw new Error('The draft changed before the edits could be saved.');
        edit.revision = saved.revision;
        if (edit.version === version) { this.edit = null; return; }
      }
    } catch (error) {
      if (this.edit === edit) edit.failed = error instanceof Error ? error.message : String(error);
      throw error;
    }
  }
}
