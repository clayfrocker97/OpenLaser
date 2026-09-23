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
  import { COMMON_SHEETS, fits, oriented } from '../lib/sheet-sizes';
  let { onclose, onselected }: { onclose: () => void; onselected: () => void } = $props();
  const stock = server.doc?.draft?.nesting?.stock;
  const bed = frameOf(server.doc!).bed;
  let width = $state(stock?.kind === 'rectangle' ? stock.bounds.max.x - stock.bounds.min.x : bed ? bed.maxX - bed.minX : 1500);
  let height = $state(stock?.kind === 'rectangle' ? stock.bounds.max.y - stock.bounds.min.y : bed ? bed.maxY - bed.minY : 1000);
  let page = $state<'new' | 'remnants' | 'drawing'>(stock?.kind === 'remnant' ? 'remnants' : 'new');
  let busy = $state(false), error = $state('');
  const bedSize = bed ? { width: bed.maxX - bed.minX, height: bed.maxY - bed.minY } : null;
  const size = $derived<[number, number]>([width, height]);
  const same = (a: [number, number], b: [number, number]) => Math.abs(a[0] - b[0]) < 1e-6 && Math.abs(a[1] - b[1]) < 1e-6;
  const presets = $derived([
    ...(bedSize ? [{ label: 'Full bed', size: [bedSize.width, bedSize.height] as [number, number] }] : []),
    ...COMMON_SHEETS.map((sheet) => ({ label: sheet.label, size: oriented(sheet.long, sheet.short, bedSize) })),
  ]);
  const saved = $derived(ui.sheetSizes.some((entry) => same(entry, size)));
  function choose(next: [number, number]): void { [width, height] = next; }
  function rotate(): void { [width, height] = [height, width]; }
  // The preview keeps the sheet's proportions inside a fixed box.
  const aspect = $derived(width > 0 && height > 0 ? width / height : 1.5);
  async function rectangle(): Promise<void> {
    busy = true; error = '';
    try { await api.setStock({ kind: 'rectangle', width, height }); ui.nestPicking = false; onselected(); onclose(); }
    catch (e) { error = explain(e); } finally { busy = false; }
  }
</script>

<Modal title="Choose nesting stock" wide onclose={() => { if (!busy) onclose(); }}>
  <div class="stock-tabs seg"><button class:on={page === 'new'} disabled={busy} onclick={() => page = 'new'}>New sheet</button><button class:on={page === 'remnants'} disabled={busy} onclick={() => page = 'remnants'}>Remnants</button><button class:on={page === 'drawing'} disabled={busy} onclick={() => page = 'drawing'}>From drawing</button></div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if page === 'new'}
    <div class="new-stock">
      <div class="sheet-preview"><div class="sheet-icon" style:aspect-ratio={aspect} style:width="min(100%, {Math.round(240 * aspect)}px)"><span>{distance(width)} × {distance(height)} {unitLabel('mm')}</span></div></div>
      <div class="sheet-form">
        <h3>A fresh rectangular sheet</h3>
        <p>Additional copies continue onto fresh sheets of this size when needed.</p>
        <div class="dimensions">
          <button disabled={busy} onclick={() => osk.number('Sheet width', width, 'mm', v => width = v)}><span>Width · X</span><strong>{quantity(width, 'mm')}</strong></button>
          <button class="rotate" disabled={busy} onclick={rotate} aria-label="Rotate the sheet: swap width and height" title="Swap width and height"><i class="ic ic-rotate"></i><span>Rotate</span></button>
          <button disabled={busy} onclick={() => osk.number('Sheet height', height, 'mm', v => height = v)}><span>Height · Y</span><strong>{quantity(height, 'mm')}</strong></button>
        </div>
        {#if !fits(size, bedSize)}<p class="warn-text" role="status">Larger than the bed ({distance(bedSize!.width)} × {distance(bedSize!.height)} {unitLabel('mm')}). Rotate it or pick a smaller size.</p>{/if}
        <h4>Common sizes</h4>
        <div class="chips">
          {#each presets as preset (preset.label)}<button class="chip" class:on={same(preset.size, size)} disabled={busy || !fits(preset.size, bedSize)} title={fits(preset.size, bedSize) ? '' : 'Larger than the bed'} onclick={() => choose(preset.size)}>{preset.label}</button>{/each}
        </div>
        <h4>Saved sizes</h4>
        <div class="chips">
          {#each ui.sheetSizes as entry (entry.join('x'))}
            <span class="saved-size"><button class="chip" class:on={same(entry, size)} disabled={busy} onclick={() => choose(entry)}>{distance(entry[0])} × {distance(entry[1])}</button><button class="forget" aria-label="Forget {distance(entry[0])} × {distance(entry[1])}" onclick={() => ui.forgetSheetSize(entry)}><i class="ic ic-x"></i></button></span>
          {/each}
          <button class="chip save" disabled={busy || saved || width <= 0 || height <= 0} onclick={() => ui.saveSheetSize(size)}>{saved ? 'Saved' : '+ Save this size'}</button>
        </div>
        <button class="btn btn-primary lg block" disabled={busy || width <= 0 || height <= 0} onclick={rectangle}>Use this sheet</button>
      </div>
    </div>
  {:else if page === 'remnants'}<SheetLibrary onchoose={async sheet => { await api.setStock({ kind: 'remnant', id: sheet.id }); ui.nestPicking = false; onselected(); onclose(); }} />
  {:else}<div class="drawing-stock"><h3>Use an existing outline</h3><p>Select a closed outline on your drawing. It becomes the stock boundary and is removed from cutting.</p><button class="btn btn-primary lg" onclick={() => { ui.nestPicking = true; onselected(); onclose(); }}>Choose outline on drawing</button></div>{/if}
</Modal>

<style>
  .stock-tabs { width:fit-content; margin-bottom:24px; } .stock-tabs button { min-height:48px; padding:12px 22px; }
  .new-stock { display:grid; grid-template-columns:1fr 1fr; align-items:center; gap:36px; padding:20px 10px; } .sheet-preview { display:grid; place-items:center; height:260px; } .sheet-icon { display:grid; place-items:center; background:var(--accent-soft); border:2px solid var(--line); border-radius:5px; color:var(--ink-3); font-size:var(--t-base); }
  .dimensions { grid-template-columns:1fr auto 1fr !important; } .dimensions .rotate { display:grid; place-items:center; gap:4px; min-width:64px; padding:8px; text-align:center; } .dimensions .rotate span { margin:0; }
  h4 { margin:14px 0 8px; font-size:var(--t-sm); color:var(--ink-3); font-weight:600; } .chips { display:flex; flex-wrap:wrap; gap:8px; } .chip { min-height:44px; } .saved-size { display:inline-flex; } .saved-size .chip { border-top-right-radius:0; border-bottom-right-radius:0; } .forget { min-width:44px; min-height:44px; border:1px solid var(--line); border-left:0; border-radius:0 10px 10px 0; background:var(--panel-2); color:var(--ink-3); cursor:pointer; } .sheet-form .btn.lg { margin-top:18px; } .warn-text { color:var(--warn); font-size:var(--t-sm); line-height:1.4; }
  h3 { font-size:var(--t-xl); color:var(--ink); text-transform:none; letter-spacing:normal; margin:0 0 16px; } p { font-size:var(--t-base); line-height:1.7; color:var(--ink-3); } .dimensions { display:grid; grid-template-columns:1fr 1fr; gap:12px; margin:24px 0; } .dimensions button { min-height:86px; padding:14px; text-align:left; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); cursor:pointer; } .dimensions span { display:block; font-size:var(--t-sm); margin-bottom:10px; color:var(--ink-3); } .dimensions strong { font-size:var(--t-lg); } .btn.lg { min-height:56px; } .drawing-stock { padding:40px; max-width:570px; margin:auto; text-align:center; } .error { color:var(--warn); }
  @media(max-width:700px) { .new-stock { grid-template-columns:1fr; } .sheet-preview { height:140px; } }
</style>
