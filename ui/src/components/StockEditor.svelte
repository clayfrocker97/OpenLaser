<script lang="ts">
  // One rack entry or remnant: how many are on hand, which folder keeps it,
  // and removing it.
  import SheetCount from './SheetCount.svelte';
  import { quantity } from '../lib/units.svelte';
  import Modal from './Modal.svelte';
  import RemnantInspector from './RemnantInspector.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { explain, laserLabel, plural } from '../lib/format';
  import { folderPath } from '../lib/folders';
  import { sheetLabel } from '../lib/sheet-sizes';
  let { id, onclose }: { id: string; onclose: () => void } = $props();
  const library = $derived(server.doc!.library);
  const item = $derived(library.stock.find((i) => i.id === id));
  const remnant = $derived(library.remnants.find((r) => r.id === id));
  const folders = $derived(library.folders
    .map((f) => ({ id: f.id, path: folderPath(library.folders, f.id).map((p) => p.name).join(' / ') }))
    .sort((a, b) => a.path.localeCompare(b.path)));
  let busy = $state(false), error = $state(''), inspecting = $state(false), confirming = $state(false);
  // Gone from the library (removed, or a remnant put on the bed): nothing to edit.
  $effect(() => { if (!item && !remnant) onclose(); });
  async function act(action: () => Promise<unknown>): Promise<void> {
    busy = true; error = '';
    try { await action(); } catch (e) { error = explain(e); } finally { busy = false; }
  }
  const setCount = (quantity: number) => act(() => api.changeStock(id, { quantity }));
  const move = (folder: string) => act(() => item
    ? api.changeStock(id, { folder: folder || null })
    : api.remnantFolder(id, folder || null));
  // Removing takes a second tap: nothing else undoes it.
  function remove(): void {
    if (!confirming) { confirming = true; return; }
    void act(async () => { await api.removeStock(id); onclose(); });
  }
</script>

{#if item || remnant}
<Modal title={item ? `${item.material} · ${sheetLabel(item.width_mm, item.height_mm)}` : remnant!.name} {onclose}>
  <div class="stock-editor">
    <p class="muted">
      {#if item}{quantity(item.thickness_mm, 'mm')} · {laserLabel(item.laser)} · full sheets on the rack
      {:else if remnant}{remnant.material} · {quantity(remnant.thickness_mm, 'mm')} · {laserLabel(remnant.mode)} ·
        remnant, {plural(remnant.cutouts.length, 'cut area')}{/if}
    </p>
    {#if item}
      <SheetCount label="On hand" prompt="Sheets on hand" value={item.quantity} disabled={busy} onchange={setCount} />
    {/if}
    {#if folders.length}
      <h4>Kept in</h4>
      <div class="chips">
        {#each [{ id: '', path: 'No folder' }, ...folders] as f (f.id)}
          <button class="chip" class:on={((item ?? remnant)!.folder ?? '') === f.id} disabled={busy} onclick={() => move(f.id)}>{f.path}</button>
        {/each}
      </div>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      {#if item}<button class="btn btn-ghost danger" disabled={busy} onclick={remove}>{confirming ? 'Tap again to remove' : 'Remove this size'}</button>
      {:else}<button class="btn btn-ghost" disabled={busy} onclick={() => inspecting = true}>Inspect remnant</button>{/if}
      <button class="btn" onclick={onclose}>Done</button>
    </div>
  </div>
</Modal>
{/if}
{#if inspecting}<RemnantInspector {id} onclose={() => inspecting = false} onchanged={() => undefined} />{/if}

<style>
  .stock-editor { display:grid; gap:18px; min-width:min(420px, 100%); }
  .muted { color:var(--ink-3); font-size:var(--t-sm); line-height:1.6; margin:0; }
  h4 { margin:0 0 -8px; font-size:var(--t-sm); color:var(--ink-3); font-weight:600; }
  .chips { display:flex; flex-wrap:wrap; gap:8px; }
  .chip { min-height:48px; }
  .actions { display:flex; justify-content:space-between; gap:10px; }
  .actions .btn { min-height:48px; }
  .danger { color:var(--warn); }
  .error { color:var(--warn); font-size:var(--t-sm); }
</style>
