<script lang="ts">
  import { quantity as measure } from '../../lib/units.svelte';
  import { onDestroy, untrack } from 'svelte';
  import StockChooser from '../../components/StockChooser.svelte';
  import { api } from '../../api/client';
  import type { NestLive, NestSettings, NestView, NestSheetPreview, StockSource } from '../../api';
  import { frameOf } from '../../lib/frame';
  import { sheetLabel } from '../../lib/sheet-sizes';
  import { current, needed, remnantMaterial, suggestStock, suits } from '../../lib/stock-plan';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { explain, plural } from '../../lib/format';
  let { selectedContours, onselectall }: { selectedContours: number[]; onselectall: () => void } = $props();
  const draft = $derived(server.doc!.draft!);
  const effectiveSelection = $derived(selectedContours.length ? selectedContours : draft.groups.length === 1 ? draft.groups[0]! : []);
  const selected = $derived(draft.groups.filter(g => g.some(i => effectiveSelection.includes(i))).length);
  let settings = $state<NestSettings>({ spacing: 3, margin: 3, remnant_clearance: 10, rotation: 'any' });
  let quantity = $state(1), seconds = $state(10), page = $state(0);
  let result = $state<NestView | null>(null), previewPage = $state<NestSheetPreview | null>(null);
  /** The running search's arrangement, updated as it improves. */
  let live = $state<NestLive | null>(null);
  let busy = $state(false), stockOpen = $state(false), cancelling = $state(false), error = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;
  let active = true, initial = '';
  const stale = $derived(result !== null && result.revision !== draft.revision);
  /** Sheets found replace the open sheet of an unsaved set, so they are
   *  numbered from it. */
  const first = $derived(draft.sheets?.pages.every((p) => !p.job) ? draft.sheets.active : 0);
  const ready = $derived(!!result?.preview && !result.running && !stale);
  const stock = $derived(draft.nesting?.stock);
  const library = $derived(server.doc!.library);
  const bed = frameOf(server.doc!).bed;
  /** New sheets default to the draft's sheet size, else the bed. */
  const fallback = $derived<[number, number]>(stock?.kind === 'rectangle'
    ? [stock.bounds.max.x - stock.bounds.min.x, stock.bounds.max.y - stock.bounds.min.y]
    : bed ? [bed.maxX - bed.minX, bed.maxY - bed.minY] : [1500, 1000]);
  const suggested = $derived(stock?.kind === 'outline' ? [] : suggestStock({
    need: needed(draft, selected === 1 ? quantity : 1),
    remnants: library.remnants.filter((r) => suits(remnantMaterial(r), draft.recipe)),
    rack: library.stock.filter((i) => suits(i, draft.recipe)),
    fallback,
    chosen: stock?.kind === 'remnant' ? stock.reference : undefined,
  }));
  /** The operator's own list once they change the suggestion. */
  let chosen = $state<StockSource[] | null>(null);
  const plan = $derived(current(chosen ?? suggested, library.stock, library.remnants));
  /** The plan a shown result was searched with, for its sheet labels. */
  let searched = $state<StockSource[]>([]);
  /** What a source is, in words: a title and a line under it. */
  function describe(source: StockSource | undefined): { title: string; detail: string } {
    if (!source) return stock?.kind === 'outline'
      ? { title: 'Drawing outline', detail: 'Not cut' }
      : { title: 'Current sheet', detail: '' };
    if (source.kind === 'remnant') {
      const sheet = library.remnants.find((r) => r.id === source.id);
      return { title: sheet?.name ?? 'Remnant', detail: sheet
        ? `Remnant · ${sheetLabel(sheet.bounds.max.x - sheet.bounds.min.x, sheet.bounds.max.y - sheet.bounds.min.y)}`
        : 'No longer available' };
    }
    if (source.kind === 'stock') {
      const item = library.stock.find((i) => i.id === source.id);
      return item
        ? { title: sheetLabel(item.width_mm, item.height_mm), detail: `Rack · ${source.count} of ${item.quantity}` }
        : { title: 'Rack sheets', detail: 'No longer on the rack' };
    }
    return { title: sheetLabel(source.width, source.height), detail: 'New · as needed' };
  }
  /** Sheets added to the list; they replace a drawing outline, which Undo brings back. */
  async function add(source: StockSource): Promise<void> {
    if (stock?.kind === 'outline') {
      try {
        await api.setStock({ kind: 'clear' });
        ui.say('Outline replaced · Undo restores it');
      } catch (e) { error = explain(e); return; }
    }
    edit([...plan, source]);
  }
  const origin = (source: StockSource | undefined) =>
    source?.kind === 'stock' ? ' · rack' : source?.kind === 'sheet' ? ' · new' : '';
  function edit(next: StockSource[]): void {
    chosen = current(next, library.stock, library.remnants);
    invalidate();
  }
  function setCount(index: number, count: number): void {
    const source = plan[index];
    if (source?.kind !== 'stock') return;
    const onHand = library.stock.find((i) => i.id === source.id)?.quantity ?? 0;
    edit(plan.map((s, i) => i === index ? { ...source, count: Math.min(onHand, Math.max(1, Math.round(count))) } : s));
  }
  const quantityTitle = $derived(selected === 1 ? 'Selected part quantity' : `${draft.groups.length} parts on this sheet`);
  const quantityHint = $derived(
    selected === 1 ? 'Including this one' : 'Tap a part to set its quantity');
  const liveNote = $derived(live
    ? `Showing sheet ${first + live.sheet}`
    : '');
  const applyLabel = $derived(result?.sheets.length === 1 ? 'layout' : `${result?.sheets.length} sheets`);
  const rotations: [NestSettings['rotation'], string, string][] = [
    ['any', 'Dense', 'Any angle'],
    ['half_turn', 'Keep grain', '0° / 180°'],
    ['across', 'Across grain', '90° / 270°'],
    ['fixed', 'Fixed', 'No rotation'],
  ];
  $effect(() => {
    const key = `${draft.generation}/${draft.job ?? ''}/${draft.sheets?.active ?? 0}`;
    if (key === initial) return;
    initial = key;
    chosen = null;
    untrack(() => {
      settings = structuredClone(
        $state.snapshot(draft.nesting?.settings) ?? { spacing: 3, margin: 3, remnant_clearance: 10, rotation: 'any' });
    });
  });
  $effect(() => { if (selected !== 1) quantity = 1; });
  $effect(() => {
    const display = previewPage ?? (result
      ? { preview: result.preview, stock_outline: result.stock_outline, stock_cutouts: result.stock_cutouts }
      : null);
    ui.nestPreview = ready ? display?.preview ?? null : null;
    ui.nestStock = ready && display ? { outline: display.stock_outline, cutouts: display.stock_cutouts } : null;
    ui.nestLive = result?.running && !stale ? live : null;
  });
  function invalidate(): void {
    result = null; previewPage = null; live = null; error = ''; page = 0;
  }
  function number(label: string, value: number, unit: string, change: (value: number) => void): void {
    osk.number(label, value, unit, v => { change(v); invalidate(); });
  }
  function setRotation(rotation: NestSettings['rotation']): void {
    settings.rotation = rotation;
    invalidate();
  }
  function setSeconds(duration: number): void {
    seconds = duration;
    invalidate();
  }
  async function poll(id: number): Promise<void> {
    if (!active) return;
    try {
      // Only a newer arrangement comes back; the search shows at most five a second.
      const next = await api.nestStatus(id, live ? result?.live_serial : undefined);
      if (!active || result?.id !== id) return;
      result = next;
      if (next.live) live = next.live;
      if (next.running) timer = setTimeout(() => { void poll(id); }, 250);
      else { live = null; cancelling = false; error = next.error ?? ''; }
    }
    catch (e) { error = explain(e); cancelling = false; }
  }
  async function start(): Promise<void> {
    if (busy || result?.running) return;
    if (!draft.nesting && !plan.length) { stockOpen = true; return; }
    if (!Number.isInteger(quantity) || quantity < 1 || quantity > 500) {
      error = 'Enter a whole quantity from 1 to 500.';
      return;
    }
    busy = true; error = ''; ui.nestPicking = false; previewPage = null; page = 0;
    live = null;
    try {
      searched = $state.snapshot(plan);
      result = await api.nest(
        { contours: effectiveSelection, quantity, settings: $state.snapshot(settings), seconds, stock: searched },
        draft.revision);
      timer = setTimeout(() => { if (result) void poll(result.id); }, 200);
    }
    catch (e) { error = explain(e); } finally { busy = false; }
  }
  async function show(index: number): Promise<void> {
    if (!result || busy || index === page) return;
    busy = true;
    try { previewPage = await api.nestSheet(result.id, index); page = index; }
    catch (e) { error = explain(e); } finally { busy = false; }
  }
  async function cancel(): Promise<void> {
    if (!result) return;
    cancelling = true;
    try {
      await api.cancelNest(result.id);
      live = null; ui.nestPreview = null; ui.nestStock = null; ui.nestLive = null;
    }
    catch (e) { error = explain(e); cancelling = false; }
  }
  async function apply(): Promise<void> {
    if (!result || stale || busy) return;
    busy = true;
    try {
      const count = result.total, sheets = result.sheets.length;
      await api.applyNest(result.id);
      invalidate();
      ui.selectionEpoch++;
      ui.setupPanel = null;
      ui.say(`${plural(count, 'part')} on ${plural(sheets, 'sheet')} · Undo restores the previous layout`);
    }
    catch (e) { error = explain(e); } finally { busy = false; }
  }
  onDestroy(() => {
    active = false;
    clearTimeout(timer);
    ui.nestPreview = null; ui.nestStock = null; ui.nestLive = null; ui.nestPicking = false;
    if (result) void api.cancelNest(result.id).catch(() => undefined);
  });
</script>

<div class="nest-panel">
<div class="feat-head">
  <h2>{ready ? 'Review sheets' : 'Nest parts'}</h2>
  <button class="btn btn-ghost" onclick={() => ui.setupPanel = null}>Close</button>
</div>
<div class="nest-scroll">
{#if ready && result}
  <div class="result-summary">
    <strong>{plural(result.total, 'part')}</strong>
    <span>Across {plural(result.sheets.length, 'sheet')} · every requested copy placed</span>
  </div>
  <div class="preview-pages" role="group" aria-label="Nesting preview sheets">
    {#each result.sheets as sheet, i}
      <button class:on={page === i} aria-pressed={page === i} disabled={busy} onclick={() => show(i)}>
        <strong>Sheet {first + sheet.number}</strong>
        <span>{plural(sheet.parts, 'part')} · {Math.round(sheet.coverage * 100)}%</span>
        <small>{describe(searched[sheet.source]).title}{origin(searched[sheet.source])}</small>
      </button>
    {/each}
  </div>
  <p class="nest-note">After applying, switch sheets above the drawing.</p>
{:else if result?.running}
  <div class="searching" role="status">
    <strong>{cancelling ? 'Cancelling…' : 'Arranging your sheets'}</strong>
    <p>{result.placed < result.total ? `${result.placed} of ${result.total} parts placed` : 'All placed · packing tighter'}</p>
    <div class="bar" class:working={!cancelling} role="progressbar" aria-label="Parts placed"
      aria-valuenow={result.placed} aria-valuemin={0} aria-valuemax={result.total}>
      <span style:width="{result.total ? Math.max(4, result.placed / result.total * 100) : 4}%"></span>
    </div>
    <p>{liveNote}</p>
    <button class="btn lg block" disabled={cancelling} onclick={cancel}>Cancel nesting</button>
  </div>
{:else}
  <fieldset disabled={busy}>
    <div class="stock-plan">
      <div class="plan-head">
        <h3>Sheets, filled in order</h3>
        {#if chosen}<button class="btn btn-ghost" onclick={() => edit(suggested)}>Suggest</button>{/if}
      </div>
      {#each plan.length ? plan : [undefined] as source, i}
        {@const said = describe(source)}
        <div class="plan-row">
          <span class="plan-name"><strong>{said.title}</strong>{#if said.detail}<small>{said.detail}</small>{/if}</span>
          {#if source?.kind === 'stock'}
            <button class="plan-count" aria-label="Sheets to use"
              onclick={() => osk.number('Rack sheets to use', source.count, 'sheets', (v) => setCount(i, v))}>{source.count}</button>
          {/if}
          {#if source && plan.length > 1}
            <button class="plan-remove" aria-label="Leave out" onclick={() => edit(plan.filter((_, j) => j !== i))}><i class="ic ic-x"></i></button>
          {/if}
        </div>
      {/each}
      <button class="btn btn-ghost add-sheets" onclick={() => stockOpen = true}><i class="ic ic-plus"></i>Add sheets</button>
    </div>
    {#if ui.nestPicking}
      <p class="pick-hint">Tap the closed sheet outline on the drawing.<button
        class="btn btn-ghost" onclick={() => ui.nestPicking = false}>Cancel</button></p>
    {/if}
    <div class="quantity">
      <span>{quantityTitle}<small>{quantityHint}</small></span>
      {#if selected === 1}
        <button class="number" onclick={() => number('Part quantity', quantity, 'copies', v => quantity = v)}>{quantity}</button>
      {:else}
        <button class="btn btn-ghost" onclick={onselectall}>Select all</button>
      {/if}
    </div>
    <div class="distances">
      <button class="value" onclick={() => number('Distance between parts', settings.spacing, 'mm', v => settings.spacing = v)}>
        <span>Part spacing</span>
        <strong>{measure(settings.spacing, 'mm')}</strong>
      </button>
      <button class="value" onclick={() => number('Sheet edge margin', settings.margin, 'mm', v => settings.margin = v)}>
        <span>Edge margin</span>
        <strong>{measure(settings.margin, 'mm')}</strong>
      </button>
    </div>
    {#if plan.some((source) => source.kind === 'remnant') || stock?.kind === 'remnant'}
      <button
        class="value remnant-buffer"
        onclick={() => number('Extra remnant clearance', settings.remnant_clearance, 'mm', v => settings.remnant_clearance = v)}>
        <span>Extra remnant clearance<small>Old cutouts + sheet edge</small></span>
        <strong>{measure(settings.remnant_clearance, 'mm')}</strong>
      </button>
    {/if}
    <h3>Rotation &amp; grain</h3>
    <div class="rotations">
      {#each rotations as [rotation, title, detail]}
        <button aria-pressed={settings.rotation === rotation} onclick={() => setRotation(rotation)}>
          <strong>{title}</strong>
          <small>{detail}</small>
        </button>
      {/each}
    </div>
    <details class="search-options">
      <summary>Search time · {seconds} sec</summary>
      <div class="seg">
        {#each [5, 10, 30] as duration}
          <button class:on={seconds === duration} onclick={() => setSeconds(duration)}>{duration} sec</button>
        {/each}
      </div>
    </details>
  </fieldset>
  {#if stale}<p class="error">The drawing changed. Preview again.</p>{/if}
{/if}
{#if error}<p class="error" role="alert">{error}</p>{/if}
<details class="credits">
  <summary>Powered by Sparrow · credits</summary>
  <p><a href="https://github.com/JeroenGar/sparrow" target="_blank" rel="noreferrer">Sparrow</a> and
    <a href="https://github.com/JeroenGar/jagua-rs" target="_blank" rel="noreferrer">jagua-rs</a> by Jeroen Gardeyn, KU Leuven.
    <a href="/licenses/sparrow-MIT.txt" target="_blank" rel="noreferrer">MIT</a> /
    <a href="/licenses/jagua-rs-MPL-2.0.txt" target="_blank" rel="noreferrer">MPL-2.0</a>.</p>
</details>
</div>
{#if ready && result}
  <div class="nest-actions">
    <button class="btn btn-primary lg block primary" disabled={busy} onclick={apply}>Apply {applyLabel}</button>
    <button class="btn btn-ghost lg block" disabled={busy} onclick={invalidate}>Back to nesting options</button>
  </div>
{:else if !result?.running}
  <div class="nest-actions">
    <button class="btn btn-primary lg block primary" disabled={busy || ui.nestPicking} onclick={start}>
      {busy ? 'Preparing…' : 'Preview sheets'}
    </button>
  </div>
{/if}
</div>
{#if stockOpen}
  <StockChooser onclose={() => stockOpen = false} onselected={() => { chosen = null; invalidate(); }}
    taken={plan.flatMap((s) => s.kind === 'stock' ? [s.id] : [])} onadd={add} />
{/if}

<style>
  .remnant-buffer { width:100%; margin-top:10px; } .remnant-buffer small { display:block; max-width:230px; margin-top:5px; font-size:var(--t-sm); line-height:1.5; color:var(--ink-3); font-weight:400; }
  .nest-panel { flex:1; min-height:0; display:flex; flex-direction:column; gap:16px; }
  .feat-head { flex-shrink:0; }
  .feat-head button { min-height:48px; }
  .nest-scroll { flex:1; min-height:0; overflow-y:auto; padding-right:3px; }
  .nest-actions { flex-shrink:0; display:grid; gap:10px; padding-top:14px; border-top:1px solid var(--line); }
  fieldset { border:0; padding:0; margin:0; min-width:0; }
  .stock-plan h3 { margin:0; }
  .plan-head { display:flex; align-items:center; justify-content:space-between; gap:10px; margin-bottom:8px; min-height:44px; }
  .plan-head .btn { min-height:44px; }
  .plan-row { display:flex; align-items:center; gap:8px; padding:8px 0; border-top:1px solid var(--line); }
  .plan-name { flex:1; min-width:0; display:grid; gap:4px; }
  .plan-name strong { font-size:var(--t-base); overflow-wrap:anywhere; }
  .plan-name small { font-size:var(--t-sm); color:var(--ink-3); line-height:1.5; }
  .plan-count { min-width:52px; min-height:44px; border:1px solid var(--line); border-radius:9px; background:var(--panel-2); color:var(--ink); font-size:var(--t-lg); cursor:pointer; }
  .plan-remove { min-width:44px; min-height:44px; border:0; background:transparent; color:var(--ink-3); cursor:pointer; }
  .add-sheets { min-height:48px; margin-top:6px; }
  .quantity { display:flex; align-items:center; justify-content:space-between; gap:16px; margin:24px 0; font-size:var(--t-sm); } .quantity small { display:block; font-size:var(--t-sm); color:var(--ink-3); margin-top:7px; line-height:1.6; }
  .number { min-width:76px; min-height:58px; border:1px solid var(--line); background:var(--panel-2); color:var(--ink); font-size:var(--t-xl); border-radius:10px; cursor:pointer; }
  .distances, .rotations { display:grid; grid-template-columns:1fr 1fr; gap:10px; }
  .value, .rotations button { border:1px solid var(--line); border-radius:9px; background:var(--panel-2); color:var(--ink); text-align:left; padding:15px; cursor:pointer; min-height:74px; }
  .value span { display:block; color:var(--ink-3); font-size:var(--t-sm); margin-bottom:10px; }
  .value strong { font-size:var(--t-lg); }
  h3 { margin:24px 0 12px; color:var(--ink-2); text-transform:none; letter-spacing:0; font-size:var(--t-sm); }
  .rotations strong { font-size:var(--t-sm); }
  .rotations small { display:block; margin-top:8px; font-size:var(--t-sm); color:var(--ink-3); }
  .rotations button[aria-pressed="true"] { border-color:var(--accent); color:var(--accent); background:var(--accent-soft); }
  .nest-note { color:var(--ink-3); font-size:var(--t-sm); line-height:1.7; margin:16px 0; }
  .search-options summary { cursor:pointer; min-height:48px; display:flex; align-items:center; color:var(--ink-3); font-size:var(--t-sm); }
  .search-options .seg button { min-height:48px; }
  .primary { min-height:58px; }
  .result-summary { display:grid; gap:10px; padding:18px 0 24px; }
  .result-summary strong { font-size:var(--t-xl); }
  .result-summary span { font-size:var(--t-sm); line-height:1.6; color:var(--ink-3); }
  .preview-pages { display:grid; grid-template-columns:1fr 1fr; gap:10px; max-height:360px; overflow:auto; padding:2px; }
  .preview-pages button { min-height:84px; padding:14px; text-align:left; border:1px solid var(--line); border-radius:9px; background:var(--panel-2); color:var(--ink); cursor:pointer; }
  .preview-pages span, .preview-pages small { display:block; font-size:var(--t-sm); margin-top:9px; color:var(--ink-3); }
  .preview-pages .on { border-color:var(--accent); background:var(--accent-soft); }
  .searching { padding:40px 0; }
  .searching strong { font-size:var(--t-lg); }
  .searching p { color:var(--ink-3); font-size:var(--t-sm); }
  /* A sheen crosses the fill while the search runs, so a pause between arrangements never looks hung. */
  .bar { height:8px; margin:12px 0; border-radius:4px; background:var(--panel-2); overflow:hidden; }
  .bar span { position:relative; display:block; height:100%; border-radius:4px; background:var(--accent); overflow:hidden; transition:width .3s; }
  .bar.working span::after { content:''; position:absolute; inset:0; background:linear-gradient(90deg, transparent, rgba(255,255,255,.45), transparent); transform:translateX(-100%); animation:sheen 1.4s ease-in-out infinite; }
  @keyframes sheen { to { transform:translateX(100%); } }
  @media (prefers-reduced-motion: reduce) { .bar.working span::after { animation:none; } }
  .error { color:var(--warn); padding:14px 0; font-size:var(--t-sm); line-height:1.65; }
  .pick-hint { font-size:var(--t-sm); color:var(--accent); line-height:1.7; }
  .pick-hint button { min-height:44px; width:100%; margin-top:7px; }
  .credits { margin-top:22px; color:var(--ink-3); font-size:var(--t-sm); line-height:1.7; } .credits summary { cursor:pointer; padding:10px 0; } .credits a { color:inherit; text-decoration:underline; }
</style>
