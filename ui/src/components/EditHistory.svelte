<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from './Modal.svelte';
  import { api } from '../api/client';
  import { explain } from '../lib/format';
  import type { EditHistory } from '../api';
  let { onclose }: { onclose: () => void } = $props();
  let history = $state<EditHistory | null>(null);
  let busy = $state(false);
  let error = $state('');
  async function load(): Promise<void> { try { history = await api.editHistory(); } catch (e) { error = explain(e); } }
  onMount(() => { void load(); });
  async function step(back: boolean): Promise<void> {
    if (!history || busy) return;
    busy = true; error = '';
    try { await api.undoSaved(back, history.revision); await load(); }
    catch (e) { error = explain(e); await load(); }
    finally { busy = false; }
  }
</script>

<Modal title="Saved edit history" {onclose}>
  <div class="row"><button class="btn" disabled={busy || !history?.past.length} onclick={() => step(true)}>Undo</button><button class="btn" disabled={busy || !history?.future.length} onclick={() => step(false)}>Redo</button></div>
  <p class="muted">Library history. Drawing Undo is in Setup.</p>
  {#if error}<p class="warn-text" role="alert">{error}</p>{/if}
  <div class="edits">
    {#each [...(history?.future ?? [])].reverse() as edit (edit.id)}<div class="edit undone"><span>{edit.label}</span><small>undone</small></div>{/each}
    {#each [...(history?.past ?? [])].reverse() as edit (edit.id)}<div class="edit"><span>{edit.label}</span><small>{new Date(edit.at * 1000).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</small></div>{:else}<p class="muted">No saved edits yet.</p>{/each}
  </div>
</Modal>
<style>
  p { margin: 0; font-size: var(--t-sm); }
  .edits { max-height: 420px; overflow-y: auto; }
  .edit { display: flex; justify-content: space-between; gap: 18px; border-bottom: 1px solid var(--line); padding: 12px 0; font-size: var(--t-sm); }
  .edit > span { min-width: 0; overflow-wrap: anywhere; }
  small { flex: 0 0 auto; color: var(--ink-3); }
  .undone { color: var(--ink-3); }
</style>
