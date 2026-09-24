<script lang="ts">
  // Sheets on hand, by material and thickness: full sheets on the rack with
  // their counts, and the remnants of the same material beside them.
  import { quantity } from '../lib/units.svelte';
  import AddStock from './AddStock.svelte';
  import StockEditor from './StockEditor.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain, laserLabel, plural } from '../lib/format';
  import { folderPath } from '../lib/folders';
  import { sheetLabel } from '../lib/sheet-sizes';
  import { remnantMaterial } from '../lib/stock-plan';
  import type { LaserMode, SheetView, StockItem } from '../api';
  const library = $derived(server.doc!.library);
  type Group = { key: string; material: string; thickness: number; laser: LaserMode; items: StockItem[]; remnants: SheetView[] };
  const groups = $derived.by(() => {
    const map = new Map<string, Group>();
    const group = (material: string, thickness: number, laser: LaserMode) => {
      const key = `${laser}/${material.trim().toLowerCase()}/${thickness}`;
      if (!map.has(key)) map.set(key, { key, material: material.trim(), thickness, laser, items: [], remnants: [] });
      return map.get(key)!;
    };
    for (const item of library.stock) group(item.material, item.thickness_mm, item.laser).items.push(item);
    for (const sheet of library.remnants) {
      const m = remnantMaterial(sheet);
      group(m.material, m.thickness_mm, m.laser).remnants.push(sheet);
    }
    return [...map.values()].sort((a, b) => a.material.localeCompare(b.material) || a.thickness - b.thickness);
  });
  const folderName = (id: string | null) => folderPath(library.folders, id).map((f) => f.name).join(' / ');
  const size = (sheet: SheetView) => sheetLabel(sheet.bounds.max.x - sheet.bounds.min.x, sheet.bounds.max.y - sheet.bounds.min.y);
  let adding = $state(false), editing = $state<string | null>(null), busy = $state(false);
  async function step(item: StockItem, by: number): Promise<void> {
    busy = true;
    try { await api.changeStock(item.id, { quantity: Math.max(0, item.quantity + by) }); }
    catch (e) { ui.say(explain(e), true); } finally { busy = false; }
  }
</script>

<div class="stock-rack">
  <div class="rack-tools">
    <button class="btn btn-primary" onclick={() => adding = true}><i class="ic ic-plus"></i>Add sheets</button>
  </div>
  {#each groups as g (g.key)}
    <section class="rack-group">
      <h3>{g.material} · {quantity(g.thickness, 'mm')} <span class="tag {g.laser}">{laserLabel(g.laser)}</span></h3>
      {#each g.items as item (item.id)}
        <div class="rack-row" class:empty={item.quantity === 0}>
          <button class="row-main" onclick={() => editing = item.id}>
            <strong>{sheetLabel(item.width_mm, item.height_mm)}</strong>
            <small>{item.quantity ? plural(item.quantity, 'sheet') : 'None left'}{item.folder ? ` · ${folderName(item.folder)}` : ''}</small>
          </button>
          <button class="step" aria-label="One fewer" disabled={busy || item.quantity <= 0} onclick={() => step(item, -1)}><i class="ic ic-minus"></i></button>
          <span class="count">{item.quantity}</span>
          <button class="step" aria-label="One more" disabled={busy} onclick={() => step(item, 1)}><i class="ic ic-plus"></i></button>
        </div>
      {/each}
      {#each g.remnants as sheet (sheet.id)}
        <div class="rack-row">
          <button class="row-main" onclick={() => editing = sheet.id}>
            <strong>{sheet.name}</strong>
            <small>Remnant · {size(sheet)} · {plural(sheet.cutouts.length, 'cut area')}{sheet.folder ? ` · ${folderName(sheet.folder)}` : ''}</small>
          </button>
          <span class="tag">Remnant</span>
        </div>
      {/each}
    </section>
  {:else}
    <div class="empty-rack">
      <strong>Nothing on the rack yet</strong>
      <p>Add the sheets you have. Saved remnants appear here too.</p>
    </div>
  {/each}
</div>
{#if adding}<AddStock folder={null} onclose={() => adding = false} />{/if}
{#if editing}<StockEditor id={editing} onclose={() => editing = null} />{/if}

<style>
  .rack-tools { display:flex; align-items:center; justify-content:flex-end; gap:18px; margin-bottom:18px; }
  .rack-tools .btn { min-height:48px; flex-shrink:0; }
  .rack-group { margin-bottom:22px; }
  .rack-group h3 { display:flex; align-items:center; gap:10px; margin:0 0 10px; font-size:var(--t-base); color:var(--ink); text-transform:none; letter-spacing:normal; }
  .rack-row { display:flex; align-items:center; gap:8px; padding:6px 6px 6px 0; border-top:1px solid var(--line); }
  .rack-row.empty .row-main strong { color:var(--ink-3); }
  .row-main { flex:1; min-width:0; min-height:52px; display:grid; gap:4px; padding:6px 10px; border:0; background:transparent; color:var(--ink); text-align:left; cursor:pointer; border-radius:8px; }
  .row-main:hover { background:var(--panel-2); }
  .row-main strong { font-size:var(--t-base); } .row-main small { font-size:var(--t-sm); color:var(--ink-3); overflow-wrap:anywhere; }
  .step { min-width:44px; min-height:44px; border:1px solid var(--line); border-radius:9px; background:var(--panel-2); color:var(--ink); cursor:pointer; }
  .count { min-width:36px; text-align:center; font-size:var(--t-lg); font-variant-numeric:tabular-nums; }
  .empty-rack { max-width:420px; padding:50px 20px; margin:auto; text-align:center; }
  .empty-rack strong { font-size:var(--t-lg); } .empty-rack p { color:var(--ink-3); font-size:var(--t-sm); line-height:1.65; }
  @media (max-width:700px) { .rack-tools { flex-direction:column; align-items:stretch; } }
</style>
