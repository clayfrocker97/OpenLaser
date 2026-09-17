<script lang="ts">
  import { distance, quantity as measure, unitLabel } from '../../lib/units.svelte';
  import { onDestroy, untrack } from 'svelte';
  import StockChooser from '../../components/StockChooser.svelte';
  import { api } from '../../api/client';
  import type { NestSettings, NestView, NestSheetPreview } from '../../api';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { explain } from '../../lib/format';
  let { selectedContours, onselectall }: { selectedContours: number[]; onselectall: () => void } = $props();
  const draft = $derived(server.doc!.draft!);
  const effectiveSelection = $derived(selectedContours.length ? selectedContours : draft.groups.length === 1 ? draft.groups[0]! : []);
  const selected = $derived(draft.groups.filter(g => g.some(i => effectiveSelection.includes(i))).length);
  let settings = $state<NestSettings>({ spacing: 3, margin: 3, remnant_clearance: 10, rotation: 'any' });
  let quantity = $state(1), seconds = $state(10), page = $state(0);
  let result = $state<NestView | null>(null), previewPage = $state<NestSheetPreview | null>(null);
  let busy = $state(false), stockOpen = $state(false), cancelling = $state(false), error = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;
  let active = true, initial = '';
  const stale = $derived(result !== null && result.revision !== draft.revision);
  const ready = $derived(!!result?.preview && !result.running && !stale);
  const stock = $derived(draft.nesting?.stock);
  const stockName = $derived(stock?.kind === 'remnant' ? stock.name : stock?.kind === 'outline' ? 'Drawing outline' : stock?.kind === 'rectangle' ? `${distance(stock.bounds.max.x - stock.bounds.min.x)} × ${distance(stock.bounds.max.y - stock.bounds.min.y)} ${unitLabel('mm')}` : 'Choose a sheet');
  $effect(() => {
    const key = `${draft.part}/${draft.job ?? ''}/${draft.sheets?.active ?? 0}`;
    if (key === initial) return; initial = key;
    untrack(() => { settings = structuredClone($state.snapshot(draft.nesting?.settings) ?? { spacing: 3, margin: 3, remnant_clearance: 10, rotation: 'any' }); });
  });
  $effect(() => { if (selected !== 1) quantity = 1; });
  $effect(() => {
    const display = previewPage ?? (result ? { preview: result.preview, stock_outline: result.stock_outline, stock_cutouts: result.stock_cutouts } : null);
    ui.nestPreview = ready ? display?.preview ?? null : null;
    ui.nestStock = ready && display ? { outline: display.stock_outline, cutouts: display.stock_cutouts } : null;
  });
  function invalidate(): void { result = null; previewPage = null; error = ''; page = 0; }
  function number(label: string, value: number, unit: string, change: (value: number) => void): void { osk.number(label, value, unit, v => { change(v); invalidate(); }); }
  async function poll(id: number): Promise<void> {
    if (!active) return;
    try { const next = await api.nestStatus(id); if (!active || result?.id !== id) return; result = next; if (next.running) timer = setTimeout(() => { void poll(id); }, 450); else { cancelling = false; error = next.error ?? ''; } }
    catch (e) { error = explain(e); cancelling = false; }
  }
  async function start(): Promise<void> {
    if (busy || result?.running) return;
    if (!draft.nesting) { stockOpen = true; return; }
    if (!Number.isInteger(quantity) || quantity < 1 || quantity > 500) { error = 'Enter a whole quantity from 1 to 500.'; return; }
    busy = true; error = ''; ui.nestPicking = false; previewPage = null; page = 0;
    try { result = await api.nest({ contours: effectiveSelection, quantity, settings: $state.snapshot(settings), seconds }, draft.revision); timer = setTimeout(() => { if (result) void poll(result.id); }, 200); }
    catch (e) { error = explain(e); } finally { busy = false; }
  }
  async function show(index: number): Promise<void> {
    if (!result || busy || index === page) return; busy = true;
    try { previewPage = await api.nestSheet(result.id, index); page = index; } catch (e) { error = explain(e); } finally { busy = false; }
  }
  async function cancel(): Promise<void> { if (!result) return; cancelling = true; try { await api.cancelNest(result.id); ui.nestPreview = null; ui.nestStock = null; } catch (e) { error = explain(e); cancelling = false; } }
  async function apply(): Promise<void> {
    if (!result || stale || busy) return; busy = true;
    try { const count = result.total, sheets = result.sheets.length; await api.applyNest(result.id); invalidate(); ui.selectionEpoch++; ui.setupPanel = null; ui.say(`${count} parts on ${sheets} sheet${sheets === 1 ? '' : 's'} · Undo restores the previous layout`); }
    catch (e) { error = explain(e); } finally { busy = false; }
  }
  onDestroy(() => { active = false; clearTimeout(timer); ui.nestPreview = null; ui.nestStock = null; ui.nestPicking = false; if (result) void api.cancelNest(result.id).catch(() => undefined); });
</script>

<div class="nest-panel">
<div class="feat-head"><h2>{ready ? 'Review sheets' : 'Nest parts'}</h2><button class="btn btn-ghost" onclick={() => ui.setupPanel = null}>Close</button></div>
<div class="nest-scroll">
{#if ready && result}
  <div class="result-summary"><strong>{result.total} parts</strong><span>Across {result.sheets.length} sheet{result.sheets.length === 1 ? '' : 's'} · every requested copy placed</span></div>
  <div class="preview-pages" role="group" aria-label="Nesting preview sheets">{#each result.sheets as sheet, i}<button class:on={page === i} aria-pressed={page === i} disabled={busy} onclick={() => show(i)}><strong>Sheet {sheet.number}</strong><span>{sheet.parts} parts · {Math.round(sheet.coverage * 100)}%</span>{#if sheet.fresh}<small>Fresh sheet</small>{/if}</button>{/each}</div>
  <p class="nest-note">Review each sheet on the drawing. After applying, switch sheets above the canvas and save the set in one folder.</p>
{:else if result?.running}
  <div class="searching" role="status"><strong>{cancelling ? 'Cancelling…' : 'Arranging your sheets'}</strong><p>{result.placed} of {result.total} parts placed</p><progress value={result.placed} max={result.total}></progress><p>Overflow goes onto the next sheet.</p><button class="btn lg block" disabled={cancelling} onclick={cancel}>Cancel nesting</button></div>
{:else}
  <p class="feat-desc">Arrange the parts and put overflow on the next sheet.</p>
  <fieldset disabled={busy}>
    <button class="stock-card" onclick={() => stockOpen = true}><span class="eyebrow">{stock?.kind === 'remnant' ? 'Remnant stock' : 'Stock'}</span><strong>{stockName}</strong><span>{stock?.kind === 'remnant' ? `${draft.stock_cutouts.length} existing cut areas kept clear` : stock?.kind === 'outline' ? 'Reference boundary · excluded from cutting' : 'New sheets of the same size'} <b>Change ›</b></span></button>
    {#if ui.nestPicking}<p class="pick-hint">Tap the closed stock outline on the drawing.<button class="btn btn-ghost" onclick={() => ui.nestPicking = false}>Cancel</button></p>{/if}
    <div class="quantity"><span>{selected === 1 ? 'Selected part quantity' : `${draft.groups.length} parts on this sheet`}<small>{selected === 1 ? 'Total copies, including this original' : 'Tap a part on the drawing to change its quantity'}</small></span>{#if selected === 1}<button class="number" onclick={() => number('Part quantity', quantity, 'copies', v => quantity = v)}>{quantity}</button>{:else}<button class="btn btn-ghost" onclick={onselectall}>Select all</button>{/if}</div>
    <div class="distances"><button class="value" onclick={() => number('Distance between parts', settings.spacing, 'mm', v => settings.spacing = v)}><span>Part spacing</span><strong>{measure(settings.spacing, 'mm')}</strong></button><button class="value" onclick={() => number('Stock edge margin', settings.margin, 'mm', v => settings.margin = v)}><span>Edge margin</span><strong>{measure(settings.margin, 'mm')}</strong></button></div>
    {#if stock?.kind === 'remnant'}<button class="value remnant-buffer" onclick={() => number('Extra remnant clearance', settings.remnant_clearance, 'mm', v => settings.remnant_clearance = v)}><span>Extra remnant clearance<small>Old cutouts + sheet edge</small></span><strong>{measure(settings.remnant_clearance, 'mm')}</strong></button>{/if}
    <h3>Rotation &amp; grain</h3><div class="rotations">{#each [['any', 'Dense', 'Any angle'], ['half_turn', 'Keep grain', '0° / 180°'], ['across', 'Across grain', '90° / 270°'], ['fixed', 'Fixed', 'No rotation']] as [rotation, title, detail]}<button aria-pressed={settings.rotation === rotation} onclick={() => { settings.rotation = rotation as NestSettings['rotation']; invalidate(); }}><strong>{title}</strong><small>{detail}</small></button>{/each}</div>
    <p class="nest-note">Lead and kerf clearance is added. {stock?.kind === 'remnant' ? 'Overflow uses fresh rectangular sheets; this remnant is used once.' : 'Part holes stay reserved.'}</p>
    <details class="search-options"><summary>Search time · {seconds} sec</summary><div class="seg">{#each [5, 10, 30] as duration}<button class:on={seconds === duration} onclick={() => { seconds = duration; invalidate(); }}>{duration} sec</button>{/each}</div></details>
  </fieldset>
  {#if stale}<p class="error">The drawing changed. Preview again to use the current layout.</p>{/if}
{/if}
{#if error}<p class="error" role="alert">{error}</p>{/if}
<details class="credits"><summary>Powered by Sparrow · credits</summary><p><a href="https://github.com/JeroenGar/sparrow" target="_blank" rel="noreferrer">Sparrow</a> and <a href="https://github.com/JeroenGar/jagua-rs" target="_blank" rel="noreferrer">jagua-rs</a> by Jeroen Gardeyn, KU Leuven. <a href="/licenses/sparrow-MIT.txt" target="_blank" rel="noreferrer">MIT</a> / <a href="/licenses/jagua-rs-MPL-2.0.txt" target="_blank" rel="noreferrer">MPL-2.0</a>.</p></details>
</div>
{#if ready && result}<div class="nest-actions"><button class="btn btn-primary lg block primary" disabled={busy} onclick={apply}>Apply {result.sheets.length === 1 ? 'layout' : `${result.sheets.length} sheets`}</button><button class="btn btn-ghost lg block" disabled={busy} onclick={invalidate}>Back to nesting options</button></div>
{:else if !result?.running}<div class="nest-actions"><button class="btn btn-primary lg block primary" disabled={busy || ui.nestPicking} onclick={start}>{busy ? 'Preparing…' : 'Preview sheets'}</button></div>{/if}
</div>
{#if stockOpen}<StockChooser onclose={() => stockOpen = false} onselected={invalidate} />{/if}

<style>
  .remnant-buffer { width:100%; margin-top:10px; } .remnant-buffer small { display:block; max-width:230px; margin-top:5px; font-size:11px; line-height:1.5; color:var(--ink-3); font-weight:400; }
  .nest-panel { flex:1; min-height:0; display:flex; flex-direction:column; gap:16px; } .feat-head { flex-shrink:0; } .feat-head button { min-height:48px; } .nest-scroll { flex:1; min-height:0; overflow-y:auto; padding-right:3px; } .feat-desc { margin:0 0 16px; } .nest-actions { flex-shrink:0; display:grid; gap:10px; padding-top:14px; border-top:1px solid var(--line); }
  fieldset { border:0; padding:0; margin:0; min-width:0; } .stock-card { width:100%; text-align:left; padding:17px; background:var(--panel-2); border:1px solid var(--line); border-radius:11px; color:var(--ink); cursor:pointer; display:grid; gap:9px; }
  .stock-card > strong { font-size:18px; } .stock-card > span:last-child { color:var(--ink-3); font-size:11px; line-height:1.7; } .stock-card b { display:block; color:var(--accent); margin-top:2px; } .eyebrow { font-size:10px; text-transform:uppercase; letter-spacing:.1em; color:var(--ink-3); }
  .quantity { display:flex; align-items:center; justify-content:space-between; gap:16px; margin:24px 0; font-size:13px; } .quantity small { display:block; font-size:11px; color:var(--ink-3); margin-top:7px; line-height:1.6; }
  .number { min-width:76px; min-height:58px; border:1px solid var(--line); background:var(--panel-2); color:var(--ink); font-size:24px; border-radius:10px; cursor:pointer; }
  .distances, .rotations { display:grid; grid-template-columns:1fr 1fr; gap:10px; } .value, .rotations button { border:1px solid var(--line); border-radius:9px; background:var(--panel-2); color:var(--ink); text-align:left; padding:15px; cursor:pointer; min-height:74px; } .value span { display:block; color:var(--ink-3); font-size:11px; margin-bottom:10px; } .value strong { font-size:17px; }
  h3 { margin:24px 0 12px; color:var(--ink-2); text-transform:none; letter-spacing:0; font-size:13px; } .rotations strong { font-size:13px; } .rotations small { display:block; margin-top:8px; font-size:11px; color:var(--ink-3); } .rotations button[aria-pressed="true"] { border-color:var(--accent); color:var(--accent); background:var(--accent-soft); }
  .nest-note { color:var(--ink-3); font-size:11px; line-height:1.7; margin:16px 0; } .search-options summary { cursor:pointer; min-height:48px; display:flex; align-items:center; color:var(--ink-3); font-size:12px; } .search-options .seg button { min-height:48px; } .primary { min-height:58px; }
  .result-summary { display:grid; gap:10px; padding:18px 0 24px; } .result-summary strong { font-size:28px; } .result-summary span { font-size:13px; line-height:1.6; color:var(--ink-3); } .preview-pages { display:grid; grid-template-columns:1fr 1fr; gap:10px; max-height:360px; overflow:auto; padding:2px; } .preview-pages button { min-height:84px; padding:14px; text-align:left; border:1px solid var(--line); border-radius:9px; background:var(--panel-2); color:var(--ink); cursor:pointer; } .preview-pages span, .preview-pages small { display:block; font-size:11px; margin-top:9px; color:var(--ink-3); } .preview-pages .on { border-color:var(--accent); background:var(--accent-soft); }
  .searching { padding:40px 0; } .searching strong { font-size:21px; } .searching p { color:var(--ink-3); font-size:13px; } progress { width:100%; accent-color:var(--accent); } .error { color:var(--warn); padding:14px 0; font-size:13px; line-height:1.65; } .pick-hint { font-size:12px; color:var(--accent); line-height:1.7; } .pick-hint button { min-height:44px; width:100%; margin-top:7px; }
  .credits { margin-top:22px; color:var(--ink-3); font-size:10px; line-height:1.7; } .credits summary { cursor:pointer; padding:10px 0; } .credits a { color:inherit; text-decoration:underline; }
</style>
