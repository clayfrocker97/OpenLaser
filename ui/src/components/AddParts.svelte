<script lang="ts">
  // Adds library parts to the open job, beside what is on the sheet, as one
  // undoable edit. Parts already in the job are copied on the sheet instead.
  import Modal from './Modal.svelte';
  import Preview from '../mobile/Preview.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain, size } from '../lib/format';
  import { fuzzyScore } from '../lib/search';
  import { togglePick } from '../lib/job-parts';

  let { onclose }: { onclose: () => void } = $props();
  const doc = $derived(server.doc!);
  const inJob = $derived(new Set(doc.draft?.parts.map((p) => p.id) ?? []));
  let search = $state('');
  let picks = $state<string[]>([]);
  let busy = $state(false);
  const parts = $derived.by(() => {
    const q = search.trim();
    const all = [...doc.library.parts].sort((a, b) => Number(inJob.has(a.id)) - Number(inJob.has(b.id)) || b.updated - a.updated);
    if (!q) return all;
    return all.map((p) => [fuzzyScore(`${p.name} ${p.tags.join(' ')} ${p.file_name}`, q), p] as const).filter(([score]) => score > 0).sort((a, b) => b[0] - a[0]).map(([, p]) => p);
  });

  async function add(): Promise<void> {
    if (busy || !picks.length) return;
    busy = true;
    try {
      await api.addParts(picks);
      ui.say(`Added ${picks.length} part${picks.length === 1 ? '' : 's'} beside the sheet · Undo takes them off`);
      onclose();
    } catch (error) {
      ui.say(explain(error), true);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="Add parts to this job" wide onclose={() => { if (!busy) onclose(); }}>
  <p class="muted">Picked parts go beside the sheet in the order picked, each its own shape. Nest parts can arrange them afterwards.</p>
  <label class="search"><i class="ic ic-search"></i><input type="search" aria-label="Search parts" placeholder="Search parts" bind:value={search} /></label>
  <div class="add-list" role="group" aria-label="Library parts">
    {#each parts as part (part.id)}
      {@const already = inJob.has(part.id)}
      {@const pick = picks.indexOf(part.id)}
      <button class="add-row" class:on={pick >= 0} disabled={already || busy} aria-pressed={pick >= 0} onclick={() => (picks = togglePick(picks, part.id))}>
        <span class="mark" class:on={pick >= 0} aria-hidden="true">{pick >= 0 ? pick + 1 : ''}</span>
        <Preview outline={part.outline} label={part.name} small />
        <span class="name"><strong>{part.name}</strong><small>{already ? 'Already in this job' : `${size(part.bounds)} · ${part.contours} paths`}</small></span>
      </button>
    {:else}
      <p class="muted">No parts match.</p>
    {/each}
  </div>
  <div class="actions">
    <button class="btn btn-ghost lg" disabled={busy} onclick={onclose}>Cancel</button>
    <button class="btn btn-primary lg" disabled={busy || !picks.length} onclick={add}>{picks.length ? `Add ${picks.length} part${picks.length === 1 ? '' : 's'}` : 'Add parts'}</button>
  </div>
</Modal>

<style>
  .muted { margin: 0 0 12px; }
  .search { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
  .search input { flex: 1; min-width: 0; min-height: 48px; }
  .add-list { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 8px; max-height: min(52vh, 520px); overflow-y: auto; padding: 2px; }
  .add-row { display: grid; grid-template-columns: 32px 64px minmax(0, 1fr); align-items: center; gap: 10px; min-height: 76px; padding: 6px 10px; text-align: left; border: 1px solid var(--line); border-radius: 12px; background: var(--panel); color: var(--ink); cursor: pointer; }
  .add-row.on { border-color: var(--accent); background: var(--accent-soft); }
  .add-row:disabled { opacity: .5; cursor: default; }
  .mark { width: 32px; height: 32px; border-radius: 50%; border: 2px solid var(--ink-3); display: grid; place-items: center; font-weight: 700; font-size: var(--t-sm); }
  .mark.on { background: var(--accent); border-color: var(--accent); color: #fff; }
  .name { display: grid; gap: 2px; min-width: 0; }
  .name strong, .name small { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .name small { color: var(--ink-3); font-size: var(--t-sm); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
</style>
