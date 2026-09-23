<script lang="ts">
  import { distance, quantity, unitLabel } from '../lib/units.svelte';
  import Modal from './Modal.svelte';
  import SheetLibrary from './SheetLibrary.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain } from '../lib/format';
  import { frameOf } from '../lib/frame';
  let { onclose, onselected }: { onclose: () => void; onselected: () => void } = $props();
  const stock = server.doc?.draft?.nesting?.stock;
  const bed = frameOf(server.doc!).bed;
  let width = $state(stock?.kind === 'rectangle' ? stock.bounds.max.x - stock.bounds.min.x : bed ? bed.maxX - bed.minX : 1500);
  let height = $state(stock?.kind === 'rectangle' ? stock.bounds.max.y - stock.bounds.min.y : bed ? bed.maxY - bed.minY : 1000);
  let page = $state<'new' | 'remnants' | 'drawing'>(stock?.kind === 'remnant' ? 'remnants' : 'new');
  let busy = $state(false), error = $state('');
  async function rectangle(): Promise<void> {
    busy = true; error = '';
    try { await api.setStock({ kind: 'rectangle', width, height }); ui.nestPicking = false; onselected(); onclose(); }
    catch (e) { error = explain(e); } finally { busy = false; }
  }
</script>

<Modal title="Choose nesting stock" wide onclose={() => { if (!busy) onclose(); }}>
  <div class="stock-tabs seg"><button class:on={page === 'new'} disabled={busy} onclick={() => page = 'new'}>New sheet</button><button class:on={page === 'remnants'} disabled={busy} onclick={() => page = 'remnants'}>Remnants</button><button class:on={page === 'drawing'} disabled={busy} onclick={() => page = 'drawing'}>From drawing</button></div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if page === 'new'}<div class="new-stock"><div class="sheet-icon"><span>{distance(width)} × {distance(height)} {unitLabel('mm')}</span></div><div><h3>A fresh rectangular sheet</h3><p>Additional copies continue onto fresh sheets of this size when needed.</p><div class="dimensions"><button disabled={busy} onclick={() => osk.number('Sheet width', width, 'mm', v => width = v)}><span>Width</span><strong>{quantity(width, 'mm')}</strong></button><button disabled={busy} onclick={() => osk.number('Sheet height', height, 'mm', v => height = v)}><span>Height</span><strong>{quantity(height, 'mm')}</strong></button></div><button class="btn btn-primary lg block" disabled={busy || width <= 0 || height <= 0} onclick={rectangle}>Use this sheet</button></div></div>
  {:else if page === 'remnants'}<SheetLibrary onchoose={async sheet => { await api.setStock({ kind: 'remnant', id: sheet.id }); ui.nestPicking = false; onselected(); onclose(); }} />
  {:else}<div class="drawing-stock"><h3>Use an existing outline</h3><p>Select a closed outline on your drawing. It becomes the stock boundary and is removed from cutting.</p><button class="btn btn-primary lg" onclick={() => { ui.nestPicking = true; onselected(); onclose(); }}>Choose outline on drawing</button></div>{/if}
</Modal>

<style>
  .stock-tabs { width:fit-content; margin-bottom:24px; } .stock-tabs button { min-height:48px; padding:12px 22px; }
  .new-stock { display:grid; grid-template-columns:1fr 1fr; align-items:center; gap:36px; padding:20px 10px; } .sheet-icon { display:grid; place-items:center; min-height:240px; margin:15px; background:var(--accent-soft); border:2px solid var(--line); border-radius:5px; color:var(--ink-3); font-size:var(--t-base); }
  h3 { font-size:var(--t-xl); color:var(--ink); text-transform:none; letter-spacing:normal; margin:0 0 16px; } p { font-size:var(--t-base); line-height:1.7; color:var(--ink-3); } .dimensions { display:grid; grid-template-columns:1fr 1fr; gap:12px; margin:24px 0; } .dimensions button { min-height:86px; padding:14px; text-align:left; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); cursor:pointer; } .dimensions span { display:block; font-size:var(--t-sm); margin-bottom:10px; color:var(--ink-3); } .dimensions strong { font-size:var(--t-lg); } .btn.lg { min-height:56px; } .drawing-stock { padding:40px; max-width:570px; margin:auto; text-align:center; } .error { color:var(--warn); }
  @media(max-width:700px) { .new-stock { grid-template-columns:1fr; } .sheet-icon { min-height:130px; } }
</style>
