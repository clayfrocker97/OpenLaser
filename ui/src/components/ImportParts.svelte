<script lang="ts">
  import Modal from './Modal.svelte';
  import CreateText from './CreateText.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain } from '../lib/format';

  let importing = $state<{ done: number; total: number } | null>(null);
  let failed = $state<string[]>([]);
  let notes = $state<string[]>([]);
  let textOpen = $state(false);
  let cancelled = false;
  async function importFiles(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const files = [...(input.files ?? [])].filter((f) => /\.(dxf|svg)$/i.test(f.name));
    input.value = '';
    if (!files.length) { ui.say('Choose DXF or SVG files.', true); return; }
    importing = { done: 0, total: files.length };
    cancelled = false;
    let added = 0, existing = 0;
    const errors: string[] = [];
    const warnings: string[] = [];
    const known = new Set(server.doc?.library.parts.map((p) => p.id));
    for (const file of files) {
      if (cancelled) break;
      try {
        const { id, warnings: messages = [] } = await api.importPart(file.name, await file.arrayBuffer());
        warnings.push(...messages.map((message) => `${file.name}: ${message}`));
        if (known.has(id)) existing += 1; else { added += 1; known.add(id); }
        ui.selected = id;
      } catch (error) { errors.push(`${file.webkitRelativePath || file.name}: ${explain(error)}`); }
      importing.done += 1;
    }
    const remaining = importing.total - importing.done;
    importing = null;
    failed = errors;
    notes = warnings;
    ui.say(`${added} added · ${existing} already in library${errors.length ? ` · ${errors.length} failed` : ''}${remaining ? ` · ${remaining} not imported` : ''}`);
  }
</script>

<div class="import-parts">
  <label class="btn btn-primary">{importing ? `${importing.done} / ${importing.total}` : '+ DXF / SVG'}<input type="file" multiple accept=".dxf,.svg" hidden disabled={!!importing} onchange={importFiles}></label>
  <label class="btn btn-ghost">Import folder<input type="file" multiple webkitdirectory accept=".dxf,.svg" hidden disabled={!!importing} onchange={importFiles}></label>
  <button class="btn btn-ghost" disabled={!!importing} onclick={() => (textOpen = true)}>+ Text</button>
  {#if importing}<button class="btn btn-ghost" onclick={() => (cancelled = true)}>Stop importing</button>{/if}
</div>
{#if failed.length}<Modal title="Files that did not import" onclose={() => (failed = [])}><div class="import-log">{failed.join('\n')}</div><button class="btn btn-primary" onclick={() => (failed = [])}>Close</button></Modal>{/if}
{#if notes.length && !failed.length}<Modal title="Import notes" onclose={() => (notes = [])}><div class="import-log">{notes.join('\n')}</div><button class="btn btn-primary" onclick={() => (notes = [])}>Close</button></Modal>{/if}
{#if textOpen}<CreateText onclose={() => (textOpen = false)} />{/if}

<style>
  .import-parts { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  label { cursor: pointer; white-space: nowrap; }
</style>
