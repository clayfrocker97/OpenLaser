<script lang="ts">
  import Modal from './Modal.svelte';
  import ImportReview from './ImportReview.svelte';
  import { api } from '../api/client';
  import type { ImportOptions, ImportReview as Review } from '../api';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain } from '../lib/format';
  import { needsReview } from '../lib/import-review';
  import { macMetadata } from '../lib/files';

  type Choice = 'import' | 'skip' | 'stop';
  interface Reviewing { file: File; bytes: ArrayBuffer; review: Review; options: ImportOptions; busy: boolean; error: string; resolve: (choice: Choice) => void }

  let importing = $state<{ done: number; total: number } | null>(null);
  let failed = $state<string[]>([]);
  let notes = $state<string[]>([]);
  let reviewing = $state<Reviewing | null>(null);
  let cancelled = false;

  // A file with a choice to make, a repair or a problem is shown first; a
  // clean one imports straight away.
  async function reviewed(file: File, bytes: ArrayBuffer): Promise<{ choice: Choice; options: ImportOptions; review: Review }> {
    const review = await api.reviewPart(file.name, bytes);
    if (!needsReview(review)) return { choice: 'import', options: {}, review };
    const choice = await new Promise<Choice>((resolve) => { reviewing = { file, bytes, review, options: {}, busy: false, error: '', resolve }; });
    const { options, review: last } = reviewing ?? { options: {}, review };
    reviewing = null;
    return { choice, options, review: last };
  }

  async function rereview(options: ImportOptions): Promise<void> {
    const current = reviewing;
    if (!current || current.busy) return;
    current.busy = true;
    current.error = '';
    try {
      const review = await api.reviewPart(current.file.name, current.bytes, options);
      if (reviewing === current) { current.review = review; current.options = options; }
    } catch (error) {
      if (reviewing === current) current.error = explain(error);
    } finally {
      current.busy = false;
    }
  }

  async function importFiles(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const files = [...(input.files ?? [])].filter((f) => /\.(dxf|svg)$/i.test(f.name) && !macMetadata(f.webkitRelativePath || f.name));
    input.value = '';
    if (!files.length) { ui.say('Choose DXF or SVG files.', true); return; }
    importing = { done: 0, total: files.length };
    cancelled = false;
    let added = 0, existing = 0, skipped = 0;
    const errors: string[] = [];
    const warnings: string[] = [];
    const known = new Set(server.doc?.library.parts.map((p) => p.id));
    for (const file of files) {
      if (cancelled) break;
      try {
        const bytes = await file.arrayBuffer();
        const { choice, options, review } = await reviewed(file, bytes);
        if (choice === 'stop') { cancelled = true; break; }
        if (choice === 'skip') {
          skipped += 1;
        } else {
          const { id, warnings: messages = [] } = await api.importPart(file.name, bytes, options);
          // A reviewed file's notices were read in its review.
          if (!needsReview(review)) warnings.push(...messages.map((message) => `${file.name}: ${message}`));
          if (known.has(id)) existing += 1; else { added += 1; known.add(id); }
          ui.selected = id;
        }
      } catch (error) { errors.push(`${file.webkitRelativePath || file.name}: ${explain(error)}`); }
      importing.done += 1;
    }
    const remaining = importing.total - importing.done;
    importing = null;
    failed = errors;
    notes = warnings;
    ui.say(`${added} added · ${existing} already in library${skipped ? ` · ${skipped} skipped` : ''}${errors.length ? ` · ${errors.length} failed` : ''}${remaining ? ` · ${remaining} not imported` : ''}`);
  }
</script>

<div class="import-parts">
  <label class="btn btn-primary">{importing ? `${importing.done} / ${importing.total}` : '+ DXF / SVG'}<input type="file" multiple accept=".dxf,.svg" hidden disabled={!!importing} onchange={importFiles}></label>
  <label class="btn btn-ghost">Import folder<input type="file" multiple webkitdirectory accept=".dxf,.svg" hidden disabled={!!importing} onchange={importFiles}></label>
  {#if importing}<button class="btn btn-ghost" onclick={() => (cancelled = true)}>Stop importing</button>{/if}
</div>
{#if reviewing}
  {@const current = reviewing}
  <ImportReview review={current.review} options={current.options} busy={current.busy} error={current.error}
    remaining={importing ? importing.total - importing.done - 1 : 0}
    onoptions={rereview}
    onimport={() => current.resolve('import')}
    onskip={() => { if (!current.busy) current.resolve('skip'); }}
    onstop={() => current.resolve('stop')} />
{/if}
{#if failed.length}<Modal title="Files that did not import" onclose={() => (failed = [])}><div class="import-log">{failed.join('\n')}</div><button class="btn btn-primary" onclick={() => (failed = [])}>Close</button></Modal>{/if}
{#if notes.length && !failed.length}<Modal title="Import notes" onclose={() => (notes = [])}><div class="import-log">{notes.join('\n')}</div><button class="btn btn-primary" onclick={() => (notes = [])}>Close</button></Modal>{/if}

<style>
  .import-parts { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  label { cursor: pointer; white-space: nowrap; }
</style>
