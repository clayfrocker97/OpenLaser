<script lang="ts">
  // A sheet size: typed, rotated, or picked from common and saved sizes.
  import { quantity, unitLabel } from '../lib/units.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain } from '../lib/format';
  import { frameOf } from '../lib/frame';
  import { COMMON_SHEETS, fits, oriented, side } from '../lib/sheet-sizes';
  let { width = $bindable(), height = $bindable(), disabled = false }:
    { width: number; height: number; disabled?: boolean } = $props();
  const bed = frameOf(server.doc!).bed;
  const bedSize = bed ? { width: bed.maxX - bed.minX, height: bed.maxY - bed.minY } : null;
  const size = $derived<[number, number]>([width, height]);
  const same = (a: [number, number], b: [number, number]) => Math.abs(a[0] - b[0]) < 1e-6 && Math.abs(a[1] - b[1]) < 1e-6;
  const presets = $derived([
    ...(bedSize ? [{ label: 'Full bed', size: [bedSize.width, bedSize.height] as [number, number] }] : []),
    ...COMMON_SHEETS.map((sheet) => ({ label: sheet.label, size: oriented(sheet.long, sheet.short, bedSize) })),
  ]);
  // Saved sizes live on the machine, so every screen offers the same list.
  const savedSizes = $derived<Array<[number, number]>>((server.doc?.sheet_sizes ?? []).map((s) => [s.width_mm, s.height_mm]));
  const saved = $derived(savedSizes.some((entry) => same(entry, size)));
  const asSheet = ([width_mm, height_mm]: [number, number]) => ({ width_mm, height_mm });
  async function saveSizes(next: Array<[number, number]>): Promise<void> {
    const expected = $state.snapshot(server.doc?.sheet_sizes ?? []);
    try { await api.saveSheetSizes(next.map(asSheet), expected); }
    catch (e) { ui.say(explain(e), true); }
  }
  const remember = () => saveSizes([...savedSizes, [width, height]]);
  const forget = (entry: [number, number]) => saveSizes(savedSizes.filter((s) => !same(s, entry)));
  function choose(next: [number, number]): void { [width, height] = next; }
  function rotate(): void { [width, height] = [height, width]; }
</script>

<div class="dimensions">
  <button {disabled} onclick={() => osk.number('Sheet width', width, 'mm', v => width = v)}>
    <span>Width · X</span><strong>{quantity(width, 'mm')}</strong>
  </button>
  <button class="rotate" {disabled} onclick={rotate} aria-label="Rotate the sheet: swap width and height" title="Swap width and height">
    <i class="ic ic-rotate"></i><span>Rotate</span>
  </button>
  <button {disabled} onclick={() => osk.number('Sheet height', height, 'mm', v => height = v)}>
    <span>Height · Y</span><strong>{quantity(height, 'mm')}</strong>
  </button>
</div>
{#if !fits(size, bedSize)}
  <p class="warn-text" role="status">
    Larger than the bed ({side(bedSize!.width)} × {side(bedSize!.height)} {unitLabel('mm')}).
    Rotate it or pick a smaller size.
  </p>
{/if}
<h4>Common sizes</h4>
<div class="chips">
  {#each presets as preset (preset.label)}
    <button
      class="chip"
      class:on={same(preset.size, size)}
      disabled={disabled || !fits(preset.size, bedSize)}
      title={fits(preset.size, bedSize) ? '' : 'Larger than the bed'}
      onclick={() => choose(preset.size)}
    >{preset.label}</button>
  {/each}
</div>
<h4>Saved sizes</h4>
<div class="chips">
  {#each savedSizes as entry (entry.join('x'))}
    <span class="saved-size">
      <button class="chip" class:on={same(entry, size)} {disabled} onclick={() => choose(entry)}>
        {side(entry[0])} × {side(entry[1])}
      </button>
      <button class="forget" aria-label="Forget {side(entry[0])} × {side(entry[1])}" onclick={() => forget(entry)}>
        <i class="ic ic-x"></i>
      </button>
    </span>
  {/each}
  <button class="chip save" disabled={disabled || saved || width <= 0 || height <= 0} onclick={remember}>
    {saved ? 'Saved' : '+ Save this size'}
  </button>
</div>

<style>
  .dimensions { display:grid; grid-template-columns:1fr auto 1fr; gap:12px; margin:24px 0; }
  .dimensions button { min-height:86px; padding:14px; text-align:left; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); cursor:pointer; }
  .dimensions span { display:block; font-size:var(--t-sm); margin-bottom:10px; color:var(--ink-3); }
  .dimensions strong { font-size:var(--t-lg); }
  .dimensions .rotate { display:grid; place-items:center; gap:4px; min-width:64px; padding:8px; text-align:center; } .dimensions .rotate span { margin:0; }
  h4 { margin:14px 0 8px; font-size:var(--t-sm); color:var(--ink-3); font-weight:600; }
  .chips { display:flex; flex-wrap:wrap; gap:8px; }
  .chip { min-height:44px; }
  .saved-size { display:inline-flex; }
  .saved-size .chip { border-top-right-radius:0; border-bottom-right-radius:0; }
  .forget { min-width:44px; min-height:44px; border:1px solid var(--line); border-left:0; border-radius:0 10px 10px 0; background:var(--panel-2); color:var(--ink-3); cursor:pointer; }
  .warn-text { color:var(--warn); font-size:var(--t-sm); line-height:1.4; }
</style>
