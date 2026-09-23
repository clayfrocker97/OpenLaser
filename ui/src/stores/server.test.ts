import { describe, expect, it } from 'vitest';
import { Server } from './server.svelte';
import type { Document, DraftView } from '../api';

const drawing = (revision = 1, changes: Partial<DraftView> = {}): DraftView => ({
  key: 'draft-a', parts: [{ id: 'part-a', first: 0, contours: 1 }], job: null, generation: 1, revision,
  placed: [{ source: 0, copy: 0, transform: [1, 0, 0, 1, 0, 0] }],
  groups: [[0]], zero: [0, 0], origin: [0, 0],
  preview: { contours: [], warnings: [], bounds: null },
  compiled: null, error: null, ...changes,
} as DraftView);
const pending = (revision = 2, changes: Partial<DraftView> = {}): DraftView => drawing(revision, {
  preview: null, error: 'preparing geometry', ...changes,
});
const started = () => {
  const server = new Server(), ready = drawing();
  server.apply({ draft: ready, draft_revision: ready.revision } as Document, true);
  return { server, ready };
};

describe('canvas continuity during geometry preparation', () => {
  it('retains a complete displayed scene while the live draft reports its new state', () => {
    const { server, ready } = started();
    const next = pending(2, { groups: [], placed: [], zero: [20, 30], origin: [20, 30] });
    server.apply({ draft: next, draft_revision: 2 });
    expect(server.canvasDraft).toBe(ready);
    expect(server.doc!.draft).toBe(next);
    expect(server.doc!.draft!.preview).toBeNull();
    expect(server.doc!.draft!.compiled).toBeNull();
    expect(server.doc!.draft_revision).toBe(2);
  });

  it('keeps the scene through successive edits and atomically replaces it with the ready reply', () => {
    const { server, ready } = started();
    server.applyDraft(pending(2), 2);
    server.applyDraft(pending(3, { groups: [[1]], zero: [40, 50] }), 3);
    server.apply({ message: null });
    expect(server.canvasDraft).toBe(ready);
    const replacement = drawing(4, { groups: [[1]], zero: [40, 50] });
    server.applyDraft(replacement, 4);
    expect(server.canvasDraft).toBe(replacement);
    expect(server.doc!.draft).toBe(replacement);
  });

  it('ignores late pending replies and deletions after a newer scene arrives', () => {
    const { server } = started();
    const replacement = drawing(4);
    server.applyDraft(replacement, 4);
    server.apply({ draft: pending(2), draft_revision: 2 });
    server.applyDraft(pending(3), 3);
    server.apply({ draft: null, draft_revision: 3 });
    expect(server.canvasDraft).toBe(replacement);
    expect(server.doc!.draft).toBe(replacement);
  });

  it('keeps the scene while parts are added to the same opening', () => {
    const { server, ready } = started();
    const parts = [{ id: 'part-a', first: 0, contours: 1 }, { id: 'part-b', first: 1, contours: 2 }];
    server.applyDraft(pending(2, { parts }), 2);
    expect(server.canvasDraft).toBe(ready);
  });

  it.each([
    { key: 'draft-b' }, { job: 'job-b' }, { generation: 2 },
  ])('never carries geometry into a different opening: %j', changes => {
    const { server } = started();
    const next = pending(2, changes);
    server.applyDraft(next, 2);
    expect(server.canvasDraft).toBe(next);
    expect(server.canvasDraft!.preview).toBeNull();
  });

  it('clears the scene when the draft closes and cannot resurrect it from an old reply', () => {
    const { server, ready } = started();
    server.applyDraft(pending(2), 2);
    server.apply({ draft: null, draft_revision: 3 });
    server.applyDraft(ready, 1);
    expect(server.canvasDraft).toBeNull();
    expect(server.doc!.draft).toBeNull();
  });

  it.each([null, { contours: [], warnings: [], bounds: null }])('shows the actual failed result instead of an obsolete scene', preview => {
    const { server } = started();
    server.applyDraft(pending(2), 2);
    const failed = drawing(3, { preview, error: 'invalid geometry' } as Partial<DraftView>);
    server.applyDraft(failed, 3);
    expect(server.canvasDraft).toBe(failed);
  });

  it('starts a reconnected stream from its own state even when generation numbers match', () => {
    const { server } = started();
    const next = pending(1);
    server.apply({ draft: next, draft_revision: 1 } as Document, true);
    expect(server.canvasDraft).toBe(next);
    expect(server.canvasDraft!.preview).toBeNull();
  });
});
