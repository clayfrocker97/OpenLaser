<script lang="ts">
  import { displayNumber, quantity, unitLabel } from '../../lib/units.svelte';
  import MaterialSheet from './MaterialSheet.svelte';
  import MaterialSummary from '../../components/MaterialSummary.svelte';
  import PreflightEditor from '../../components/PreflightEditor.svelte';
  import { pictureOf } from '../../lib/materials';
  import { api } from '../../api/client';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { ago, explain, laserLabel, plural, seconds, size } from '../../lib/format';
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

  function sheetKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') sheet = true;
  }

  function review(dryRun: boolean): void {
    run(async () => { if (draft.dry_run !== dryRun) await api.compile(dryRun); }, () => { ui.tab = 'run'; });
  }
</script>

<div class="job-scroll">
<div class="card2">
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

<div class="card2" role="button" tabindex="0" onclick={() => (sheet = true)} onkeydown={sheetKey}>
  <div class="card2-head"><h3>Material</h3><span class="link">Change</span></div>
  {#if recipe}
    {@const art = pictureOf(recipe.name, recipe.photo)}
    <div class="mat-line">
      {#if art}<img class="swatch lg" src={art} alt="">{:else}<span class="swatch lg" style="background:var(--panel-2)"></span>{/if}
      <div>
        <div class="big">{recipe.name}</div>
        <div class="muted">{quantity(recipe.thickness_mm, 'mm')} · {recipe.gas} · {laserLabel(recipe.laser)}</div>
      </div>
    </div>
    <MaterialSummary source={recipe} />
  {:else}
    <div class="mat-line">
      <span class="swatch lg" style="background:var(--warn-soft)"></span>
      <div><div class="big warn-text">Choose a material</div><div class="muted">Required before running</div></div>
    </div>
  {/if}
</div>

{#if draft.calibration}<p class="correction-status">Correction off</p>{:else if draft.correction}<p class="correction-status">Matrix correction</p>{/if}

<div class="card2">
  <div class="card2-head"><h3>Machining</h3><span class="muted">{on} of {optional} on</span></div>
  <div class="muted">{#if draft.feature_source}Prefilled from <strong>{draft.feature_source.name}</strong> · {ago(draft.feature_source.at)}{:else}Defaults for this recipe{/if}</div>
</div>

<div class="card2">
  <div class="card2-head">
    <div><h3>Preflight</h3><span class="muted">{preflightLabel}</span></div>
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
{#if draft.error}<div class="warn-text" style="font-size:var(--t-sm)">{draft.error}</div>{/if}
<GasLaserCard />
</div>

<div class="stack">
  <div class="row">
    <button class="btn btn-ghost lg" onclick={save} disabled={!recipe}>{saveLabel}</button>
    <button class="btn btn-ghost lg" onclick={() => review(true)} disabled={!doc.readiness.compile.ok} title={doc.readiness.compile.reason ?? ''}>Dry run</button>
  </div>
  <button class="btn btn-primary xl block" onclick={() => review(false)} disabled={!doc.readiness.compile.ok} title={doc.readiness.compile.reason ?? ''}>Go to Run →</button>
</div>

{#if sheet}<MaterialSheet onclose={() => (sheet = false)} />{/if}
{#if adding}<AddParts onclose={() => (adding = false)} />{/if}
{#if texting}<CreateText onclose={() => (texting = false)} />{/if}
{#if checklist}<PreflightEditor scope="job" onclose={() => (checklist = false)} />{/if}

<style>
  /* The cards scroll on a short screen; saving and Go to Run stay in reach. */
  .parts-actions { display:flex; gap:8px; flex-wrap:wrap; justify-content:flex-end; }
  .job-scroll { flex:1; min-height:0; overflow-y:auto; overscroll-behavior:contain; display:flex; flex-direction:column; gap:12px; margin:-2px; padding:2px; }
  .stack { flex-shrink:0; }
  .correction-status { padding:10px 12px; font-size:var(--t-sm); color:var(--ink-3); border-left:2px solid var(--accent); }
  .job-parts { list-style:none; margin:0; padding:0; display:grid; gap:6px; max-height:220px; overflow-y:auto; }
  .job-part { width:100%; min-height:48px; display:flex; align-items:center; justify-content:space-between; gap:12px; padding:8px 12px; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); text-align:left; cursor:pointer; }
  .job-part span { font-weight:600; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .job-part small { flex:none; color:var(--ink-3); font-size:var(--t-sm); }
</style>
