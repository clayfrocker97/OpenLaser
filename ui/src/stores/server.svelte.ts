// The server's document, merged from the event stream. Sections the
// stream leaves out are unchanged since the last event.
import type { Document, DraftView } from '../api';

export class Server {
  // Sections are replaced by server replies; geometry is never edited here.
  doc = $state.raw<Document | null>(null);
  /** Complete scene kept on screen during preparation. Actions use doc.draft. */
  canvasDraft = $state.raw<DraftView | null>(null);
  /** Whether the event stream is up. */
  link = $state(false);

  apply = (partial: Partial<Document>, first = false) => {
    if (this.doc === null || first) {
      // The first event of a stream is the whole document.
      this.publish(partial as Document, true);
      return;
    }
    if (partial.draft_revision !== undefined && partial.draft_revision < this.doc.draft_revision) {
      partial = { ...partial, draft: this.doc.draft, draft_revision: this.doc.draft_revision };
    }
    this.publish({ ...this.doc, ...partial });
  };

  applyDraft(draft: DraftView | null, revision: number): void {
    if (!this.doc || this.doc.draft_revision > revision) return;
    this.publish({ ...this.doc, draft, draft_revision: revision });
  }

  private publish(doc: Document, first = false): void {
    const previous = first ? null : this.canvasDraft;
    const next = doc.draft;
    // Groups, placements and the drawing origin must arrive with their
    // preview. Retain only this opening's scene, never another job's.
    const preparing = next?.error === 'preparing geometry' && !next.preview;
    const sameOpening = previous && next && previous.generation === next.generation
      && previous.part === next.part && previous.job === next.job;
    this.canvasDraft = preparing && sameOpening && previous.preview ? previous : next;
    this.doc = doc;
  }

  setLink = (up: boolean) => {
    this.link = up;
  };
}

export const server = new Server();
