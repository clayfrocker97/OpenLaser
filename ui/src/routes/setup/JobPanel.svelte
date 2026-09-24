<script lang="ts">
  import { plain } from '../../lib/plain';
  import { displayNumber, quantity, unitLabel } from '../../lib/units.svelte';
  import MaterialSheet from './MaterialSheet.svelte';
  import LayerRecipeChooser from './LayerRecipeChooser.svelte';
  import MaterialSummary from '../../components/MaterialSummary.svelte';
  import PreflightEditor from '../../components/PreflightEditor.svelte';
  import { pictureOf } from '../../lib/materials';
  import { api } from '../../api/client';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { explain, laserLabel, plural, recipeLabel, seconds, size } from '../../lib/format';
  import { layerColor } from '../../lib/drawing-layers';
  import type { DraftLayer } from '../../api';
  import { TOOLS, isOn } from '../../lib/features';
  import AddParts from '../../components/AddParts.svelte';
  import CreateText from '../../components/CreateText.svelte';
  import GasLaserCard from '../../components/GasLaserCard.svelte';

  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft!);
  const recipe = $derived(draft.recipe);
  /** Selects a part's shapes on the drawing, when there is one to select on. */
  let { onselectpart }: { onselectpart?: (first: number, count: number) => void } = $props();
  const nameOf = (id: string) => doc.library.parts.find((p) => p.id === id)?.name ?? 'Missing part';
  let adding = $state(false);
  let texting = $state(false);
  const job = $derived(doc.library.jobs.find((j) => j.id === draft.job) ?? null);
  const on = $derived(TOOLS.filter((t) => t.optional && isOn(draft.features, t.id)).length);
  const optional = TOOLS.filter((t) => t.optional).length;

  const partsLabel = $derived(draft.parts.length === 1 ? nameOf(draft.parts[0]!.id) : `${draft.parts.length} parts`);
  const preflightLabel = $derived(
    draft.preflight.kind === 'inherit' ? 'Mode defaults'
    : draft.preflight.kind === 'off' ? 'Checklist off'
    : `${draft.preflight.steps.length} custom checks`,
  );
  const saveLabel = $derived(
    draft.sheets?.pages.some(p => !p.job) ? `Save ${plural(draft.sheets.pages.length, 'sheet')}` : 'Save job',
  );
  /** The note beside a part in the list: whether it is on this sheet, and how many copies or paths. */
  function partNote(here: number, copies: number, contours: number): string {
    if (!here) return 'Not on this sheet';
    return copies > 1 ? `${copies} copies · Select` : `${plural(contours, 'path')} · Select`;
  }

  /** Several layers to output: each needs a recipe or to be off. */
  const layersAsking = $derived(draft.layers.filter((l) => l.output).length > 1);
  const layersMissing = $derived(layersAsking && draft.layers.some((l) => l.output && !l.chosen));
  function layerHow(layer: DraftLayer): string {
    if (!layer.output) return 'Off';
    if (!layer.chosen) return layersAsking ? 'Choose a recipe' : 'Job recipe';
    return `${layer.mode === 'mark' ? 'Mark · ' : ''}${layer.recipe ? recipeLabel(layer.recipe) : 'Job recipe'}`;
  }

  let choosing = $state<DraftLayer | null>(null);
  let sheet = $state(false);
  let checklist = $state(false);

  async function run(action: () => Promise<unknown>, then?: () => void): Promise<void> {
    try { await action(); then?.(); } catch (error) { ui.say(explain(error), true); }
  }

  function save(): void {
    const batch = draft.sheets?.pages.some(p => !p.job) ?? false;
    const total = draft.sheets?.pages.length ?? 1;
    osk.text(batch ? 'Folder for numbered sheets' : 'Job name', job?.name ?? (draft.name || 'job'), (name) => {
      if (!name.trim()) return;
      run(async () => {
        if (batch) {
          await api.saveJob(name.trim());
          ui.say(`Saved ${total} numbered sheets in ${name.trim()}.`);
          return;
        }
        const review = await api.mergeReview(name.trim());
        if (review.conflicts.length) {
          ui.pendingJobName = name.trim();
          ui.modal = 'pending';
          return;
        }
        await api.saveJob(review.name);
        ui.say(`Saved job ${review.name}.`);
      });
    });
  }

  function review(dryRun: boolean): void {
    run(async () => { if (draft.dry_run !== dryRun) await api.compile(dryRun); }, () => { ui.tab = 'run'; });
  }
</script>

<div class="job-scroll">
<div class="card2 compact">
  <div class="card2-head">
    <div><h3>Parts</h3><span class="muted">{partsLabel}</span></div>
    <div class="parts-actions">
      <button class="btn btn-ghost" onclick={() => (texting = true)}>Add text</button>
      <button class="btn btn-ghost" onclick={() => (adding = true)}>Add parts</button>
    </div>
  </div>
  {#if draft.parts.length > 1}
    <ul class="job-parts">
      {#each draft.parts as part (part.id)}
        {@const here = draft.placed.filter((p) => p.source >= part.first && p.source < part.first + part.contours).length}
        {@const copies = Math.max(1, Math.round(here / Math.max(1, part.contours)))}
        <li>
          <button class="job-part" disabled={!onselectpart || !here} onclick={() => onselectpart?.(part.first, part.contours)}>
            <span>{nameOf(part.id)}</span>
            <small>{partNote(here, copies, part.contours)}</small>
          </button>
        </li>
      {/each}
    </ul>
    <p class="muted">Delete a part's shapes on the drawing to take it out of the job.</p>
  {/if}
</div>

<div class="card2 compact">
  <button class="mat-row" onclick={() => (sheet = true)}>
    {#if recipe}
      {@const art = pictureOf(recipe.name, recipe.photo)}
      {#if art}<img class="swatch" src={art} alt="">{:else}<span class="swatch" style="background:var(--panel-2)"></span>{/if}
      <span class="mat-text">
        <strong>{recipe.name}</strong>
        <small>{quantity(recipe.thickness_mm, 'mm')} · {recipe.gas} · {laserLabel(recipe.laser)}</small>
      </span>
    {:else}
      <span class="swatch" style="background:var(--warn-soft)"></span>
      <span class="mat-text"><strong class="warn-text">Choose a material</strong><small>Required before running</small></span>
    {/if}
    <span class="link">Change</span>
  </button>
  {#if recipe}<MaterialSummary source={recipe} variant="compact" />{/if}
</div>

<div class="card2 layers-card" class:needs={layersMissing}>
  <div class="card2-head"><h3>Layers</h3><button class="btn btn-ghost" onclick={() => (ui.setupPanel = 'layers')}>Edit</button></div>
  <div class="layer-lines">
    {#each draft.layers.slice(0, 4) as layer (layer.name)}
      {@const needs = layersAsking && layer.output && !layer.chosen}
      <button class="layer-line" class:off={!layer.output} class:needs onclick={() => (choosing = layer)}>
        <span class="layer-swatch" class:mark={layer.mode === 'mark'} style:--layer={layerColor(draft.layers, layer.name) ?? 'var(--cut)'}></span>
        <span class="layer-name">{layer.name}</span>
        <span class="layer-how">{needs ? 'Choose…' : layerHow(layer)}</span>
        <i class="ic ic-chev-right"></i>
      </button>
    {/each}
  </div>
  {#if draft.layers.length > 4}<button class="btn btn-ghost block" onclick={() => (ui.setupPanel = 'layers')}>{draft.layers.length - 4} more layers</button>{/if}
</div>

{#if draft.calibration}<p class="correction-status">Correction off</p>{:else if draft.correction}<p class="correction-status">Matrix correction</p>{/if}

<div class="card2 compact rows">
  <div class="info-row">
    <span><strong>Machining</strong><small>{on} of {optional} on{#if draft.feature_source} · from {draft.feature_source.name}{/if}</small></span>
    <span><strong>Preflight</strong><small>{preflightLabel}</small></span>
    <button class="btn btn-ghost" onclick={() => (checklist = true)}>Edit</button>
  </div>
</div>

<div class="stats">
  <div><b>{displayNumber(draft.preview?.length_mm, 'mm', 0)}</b><span>{unitLabel('mm')} cut</span></div>
  <div><b>{draft.compiled ? draft.compiled.pierces.length : '—'}</b><span>pierces</span></div>
  <div><b>{draft.compiled ? seconds(draft.compiled.seconds) : '—'}</b><span>est. time</span></div>
</div>
{#if draft.compiled?.plan.some((pass) => pass.omitted_cooling > 0)}
  <p class="muted">{draft.compiled.plan.reduce((n, pass) => n + pass.omitted_cooling, 0)} cooling points skipped within {quantity(0.2, 'mm')} of endpoints.</p>
{/if}
{#if draft.error}<div class="warn-text" style="font-size:var(--t-sm)">{plain(draft.error).text}</div>{/if}
{#if draft.compiled}<GasLaserCard />{/if}
</div>

<div class="stack">
  <div class="row">
    <button class="btn btn-ghost lg" onclick={save} disabled={!recipe}>{saveLabel}</button>
    <button class="btn btn-ghost lg" onclick={() => review(true)} disabled={!doc.readiness.compile.ok} title={doc.readiness.compile.reason ?? ''}>Dry run</button>
  </div>
  <button class="btn btn-primary xl block" onclick={() => review(false)} disabled={!doc.readiness.compile.ok} title={doc.readiness.compile.reason ?? ''}>Go to Run →</button>
</div>

{#if choosing}<LayerRecipeChooser layer={choosing} onclose={() => (choosing = null)} />{/if}
{#if sheet}<MaterialSheet onclose={() => (sheet = false)} />{/if}
{#if adding}<AddParts onclose={() => (adding = false)} />{/if}
{#if texting}<CreateText onclose={() => (texting = false)} />{/if}
{#if checklist}<PreflightEditor scope="job" onclose={() => (checklist = false)} />{/if}

<style>
  /* The cards scroll on a short screen; saving and Go to Run stay in reach. */
  .parts-actions { display:flex; gap:8px; flex-wrap:wrap; justify-content:flex-end; }
  .job-scroll { flex:1; min-height:0; overflow-y:auto; overscroll-behavior:contain; display:flex; flex-direction:column; gap:8px; margin:-2px; padding:2px; }
  .stack { flex-shrink:0; }
  .correction-status { padding:10px 12px; font-size:var(--t-sm); color:var(--ink-3); border-left:2px solid var(--accent); }
  .job-parts { list-style:none; margin:0; padding:0; display:grid; gap:6px; max-height:220px; overflow-y:auto; }
  .job-part {
    width:100%; min-height:48px; display:flex; align-items:center; justify-content:space-between; gap:12px; padding:8px 12px;
    border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); text-align:left; cursor:pointer;
  }
  .job-part span { font-weight:600; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .job-part small { flex:none; color:var(--ink-3); font-size:var(--t-sm); }
  .compact { padding:10px 12px; gap:8px; }
  .mat-row { display:flex; align-items:center; gap:12px; min-height:52px; padding:0; border:0; background:transparent; color:var(--ink); font:inherit; text-align:left; cursor:pointer; }
  .mat-row .swatch { flex:none; width:40px; height:40px; border-radius:8px; object-fit:cover; }
  .mat-text { flex:1; min-width:0; display:grid; gap:2px; }
  .mat-text strong { font-size:var(--t-base); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .mat-text small { font-size:var(--t-sm); color:var(--ink-3); }
  .rows { gap:0; padding-top:4px; padding-bottom:4px; }
  .info-row { display:flex; align-items:center; justify-content:space-between; gap:10px; min-height:44px; }
  .info-row > span { flex:1; }
  .info-row > span { display:grid; gap:2px; min-width:0; }
  .info-row strong { font-size:var(--t-base); }
  .info-row small { font-size:var(--t-sm); color:var(--ink-3); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .info-row .btn { min-height:44px; }
  .layers-card { padding:10px 12px; gap:8px; }
  .layers-card.needs { border-color:var(--warn); }
  .layer-lines { display:grid; gap:6px; }
  .layer-line {
    width:100%; min-height:44px; display:flex; align-items:center; gap:10px; padding:0 8px 0 12px; min-width:0;
    border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); font:inherit; text-align:left; cursor:pointer;
  }
  .layer-line.off { opacity:.55; }
  .layer-line.needs { border-color:var(--warn); }
  .layer-line.needs .layer-how { color:var(--warn); font-weight:600; }
  .layer-swatch { flex:none; width:16px; height:16px; border-radius:4px; background:var(--layer); }
  .layer-swatch.mark { background:transparent; border:2px dashed var(--layer); }
  .layer-name { font-weight:600; font-size:var(--t-base); min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .layer-how { margin-left:auto; flex:none; max-width:55%; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:var(--ink-3); font-size:var(--t-sm); }
</style>
