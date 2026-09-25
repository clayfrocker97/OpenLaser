<script lang="ts">
  // Adds full sheets to the rack: a material from the recipes, a size and
  // how many. The same material and size in the same folder adds up.
  import SheetCount from './SheetCount.svelte';
  import { quantity } from '../lib/units.svelte';
  import { untrack } from 'svelte';
  import Modal from './Modal.svelte';
  import SheetSizePicker from './SheetSizePicker.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain, laserLabel, plural } from '../lib/format';
  import { frameOf } from '../lib/frame';
  import type { LaserMode } from '../api';
  type Material = { name: string; laser: LaserMode; thickness_mm: number };
  let { material = null, folder, onclose }: { material?: Material | null; folder: string | null; onclose: () => void } = $props();
  const key = (m: Material) => `${m.laser}/${m.name.toLowerCase()}/${m.thickness_mm}`;
  // Every material a recipe cuts, once each.
  const materials = $derived([...new Map((server.doc?.library.recipes ?? [])
    .map((r) => [key(r), { name: r.name, laser: r.laser, thickness_mm: r.thickness_mm }] as const)).values()]
    .sort((a, b) => a.name.localeCompare(b.name) || a.thickness_mm - b.thickness_mm));
  const initial = untrack(() => material ?? server.doc?.draft?.recipe ?? null);
  let chosen = $state(initial ? key(initial) : '');
  const picked = $derived(materials.find((m) => key(m) === chosen) ?? (material && key(material) === chosen ? material : null));
  const names = $derived([...new Set(materials.map((m) => m.name))]);
  const lasers = $derived(new Set(materials.filter((m) => m.name === picked?.name).map((m) => m.laser)).size);
  /** A material name picks its first thickness; the thickness chips refine it. */
  function pickName(name: string): void {
    const first = materials.find((m) => m.name === name);
    if (first) chosen = key(first);
  }
  const bed = frameOf(server.doc!).bed;
  let width = $state(bed ? bed.maxX - bed.minX : 2500), height = $state(bed ? bed.maxY - bed.minY : 1250);
  let count = $state(1), busy = $state(false), error = $state('');
  const label = (m: Material) => `${m.name} · ${quantity(m.thickness_mm, 'mm')} · ${laserLabel(m.laser)}`;
  async function save(): Promise<void> {
    if (!picked) return;
    busy = true; error = '';
    try {
      await api.addStock({
        material: picked.name, thickness_mm: picked.thickness_mm, laser: picked.laser,
        width_mm: width, height_mm: height, quantity: count, folder,
      });
      ui.say(`${plural(count, 'sheet')} added to the rack`);
      onclose();
    }
    catch (e) { error = explain(e); } finally { busy = false; }
  }
</script>

<Modal title="Add sheets to the rack" wide onclose={() => { if (!busy) onclose(); }}>
  <div class="add-stock">
    {#if material}
      <p class="fixed">{label(material)}</p>
    {:else}
      <h4>Material</h4>
      <div class="chips">
        {#each names as name (name)}
          <button class="chip" class:on={picked?.name === name} disabled={busy} onclick={() => pickName(name)}>{name}</button>
        {:else}<p class="fixed">Add a recipe first.</p>{/each}
      </div>
      {#if picked}
        <h4>Thickness</h4>
        <div class="chips">
          {#each materials.filter((m) => m.name === picked!.name) as m (key(m))}
            <button class="chip" class:on={key(m) === chosen} disabled={busy} onclick={() => chosen = key(m)}>
              {quantity(m.thickness_mm, 'mm')}{lasers > 1 ? ` · ${laserLabel(m.laser)}` : ''}
            </button>
          {/each}
        </div>
      {/if}
    {/if}
    <SheetSizePicker bind:width bind:height disabled={busy} />
    <div class="count"><SheetCount label="Sheets" prompt="Sheets to add" value={count} min={1} disabled={busy} onchange={(v) => (count = v)} /></div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <button class="btn btn-primary lg block" disabled={busy || !picked || width <= 0 || height <= 0} onclick={save}>
      Add {plural(count, 'sheet')}
    </button>
  </div>
</Modal>

<style>
  .add-stock { max-width:560px; margin:auto; padding:6px; }
  h4 { margin:14px 0 8px; font-size:var(--t-sm); color:var(--ink-3); font-weight:600; }
  .chips { display:flex; flex-wrap:wrap; gap:8px; }
  .chip { min-height:48px; }
  .fixed { margin:0; font-size:var(--t-base); color:var(--ink-2); }
  .count { margin:22px 0; }
  .btn.lg { min-height:56px; }
  .error { color:var(--warn); font-size:var(--t-sm); }
</style>
