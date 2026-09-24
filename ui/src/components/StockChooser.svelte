<script lang="ts">
  // Adds a kind of sheet to the ones a nest fills: new sheets of a size,
  // sheets on the rack, a remnant, or an outline on the drawing.
  import { sheetLabel } from '../lib/sheet-sizes';
  import { untrack } from 'svelte';
  import Modal from './Modal.svelte';
  import SheetLibrary from './SheetLibrary.svelte';
  import SheetSizePicker from './SheetSizePicker.svelte';
  import AddStock from './AddStock.svelte';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { frameOf } from '../lib/frame';
  import { suits } from '../lib/stock-plan';
  import { plural } from '../lib/format';
  import type { SheetView, StockSource } from '../api';
  let { onclose, onadd, onselected, taken = [] }: {
    onclose: () => void; onadd: (source: StockSource) => void; onselected: () => void;
    /** Rack entries already in the list. */ taken?: string[];
  } = $props();
  const draft = $derived(server.doc?.draft);
  const bed = frameOf(server.doc!).bed;
  const stock = server.doc?.draft?.nesting?.stock;
  let width = $state(stock?.kind === 'rectangle' ? stock.bounds.max.x - stock.bounds.min.x : bed ? bed.maxX - bed.minX : 1500);
  let height = $state(stock?.kind === 'rectangle' ? stock.bounds.max.y - stock.bounds.min.y : bed ? bed.maxY - bed.minY : 1000);
  const rack = $derived((server.doc?.library.stock ?? [])
    .filter((i) => i.quantity > 0 && suits(i, draft?.recipe) && !taken.includes(i.id)));
  let page = $state<'new' | 'rack' | 'remnants' | 'drawing'>(untrack(() => rack.length) ? 'rack' : 'new');
  let adding = $state(false);
  // The preview keeps the sheet's proportions inside a fixed box.
  const aspect = $derived(width > 0 && height > 0 ? width / height : 1.5);
  function add(source: StockSource): void {
    onadd(source);
    onclose();
  }
  async function useRemnant(sheet: SheetView): Promise<void> { add({ kind: 'remnant', id: sheet.id }); }
  function chooseOutline(): void {
    ui.nestPicking = true;
    onselected();
    onclose();
  }
</script>

<Modal title="Add sheets to nest on" wide {onclose}>
  <div class="stock-tabs seg">
    <button class:on={page === 'rack'} onclick={() => page = 'rack'}>On the rack</button>
    <button class:on={page === 'remnants'} onclick={() => page = 'remnants'}>Remnants</button>
    <button class:on={page === 'new'} onclick={() => page = 'new'}>New sheets</button>
    <button class:on={page === 'drawing'} onclick={() => page = 'drawing'}>From drawing</button>
  </div>
  {#if page === 'new'}
    <div class="new-stock">
      <div class="sheet-preview">
        <div class="sheet-icon" style:aspect-ratio={aspect} style:width="min(100%, {Math.round(240 * aspect)}px)">
          <span>{sheetLabel(width, height)}</span>
        </div>
      </div>
      <div class="sheet-form">
        <h3>New sheets</h3>
        <SheetSizePicker bind:width bind:height />
        <button
          class="btn btn-primary lg block"
          disabled={width <= 0 || height <= 0}
          onclick={() => add({ kind: 'sheet', width, height, count: null })}>Add these sheets</button>
      </div>
    </div>
  {:else if page === 'rack'}
    <div class="rack">
      {#each rack as item (item.id)}
        <button class="rack-choice" onclick={() => add({ kind: 'stock', id: item.id, count: item.quantity })}>
          <strong>{sheetLabel(item.width_mm, item.height_mm)}</strong>
          <span>{plural(item.quantity, 'sheet')} on hand</span>
        </button>
      {:else}
        <p class="empty">{taken.length ? 'Already in the list.' : 'None on the rack.'}</p>
      {/each}
      <button class="btn btn-ghost" onclick={() => adding = true}>+ Add sheets to the rack</button>
    </div>
  {:else if page === 'remnants'}<SheetLibrary onchoose={useRemnant} />
  {:else}
    <div class="drawing-stock">
      <h3>Use an existing outline</h3>
      <p>Tap a closed outline. It becomes the sheet and is not cut.</p>
      <button class="btn btn-primary lg" onclick={chooseOutline}>Choose outline on drawing</button>
    </div>
  {/if}
</Modal>
{#if adding}
  <AddStock material={draft?.recipe ?? null} folder={draft?.job ? server.doc?.library.jobs.find((j) => j.id === draft.job)?.folder ?? null : null}
    onclose={() => adding = false} />
{/if}

<style>
  .stock-tabs { max-width:100%; overflow-x:auto; margin-bottom:24px; } .stock-tabs button { min-height:48px; padding:12px 22px; }
  .new-stock { display:grid; grid-template-columns:1fr 1fr; align-items:center; gap:36px; padding:20px 10px; }
  .sheet-preview { display:grid; place-items:center; height:260px; }
  .sheet-icon { display:grid; place-items:center; background:var(--accent-soft); border:2px solid var(--line); border-radius:5px; color:var(--ink-3); font-size:var(--t-base); }
  .sheet-form .btn.lg { margin-top:18px; }
  h3 { font-size:var(--t-xl); color:var(--ink); text-transform:none; letter-spacing:normal; margin:0 0 16px; }
  p { font-size:var(--t-base); line-height:1.7; color:var(--ink-3); }
  .btn.lg { min-height:56px; }
  .rack { display:grid; gap:10px; max-width:560px; margin:auto; padding:10px; }
  .rack-choice { display:flex; justify-content:space-between; align-items:center; gap:12px; min-height:64px; padding:14px 16px; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); text-align:left; cursor:pointer; }
  .rack-choice strong { font-size:var(--t-lg); } .rack-choice span { font-size:var(--t-sm); color:var(--ink-3); }
  .rack .btn { min-height:48px; justify-self:start; }
  .drawing-stock { padding:40px; max-width:570px; margin:auto; text-align:center; }
  @media(max-width:700px) { .new-stock { grid-template-columns:1fr; } .sheet-preview { height:140px; } }
</style>
