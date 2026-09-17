<script lang="ts">
  import { displayNumber, quantity, unitLabel } from '../../lib/units.svelte';
  import MaterialSheet from './MaterialSheet.svelte';
  import PreflightEditor from '../../components/PreflightEditor.svelte';
  import { pictureOf } from '../../lib/materials';
  import { api } from '../../api/client';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { ago, explain, laserLabel, seconds, size, value } from '../../lib/format';
  import { TOOLS, isOn } from '../../lib/features';

  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft!);
  const recipe = $derived(draft.recipe);
  const part = $derived(doc.library.parts.find((p) => p.id === draft.part) ?? null);
  const job = $derived(doc.library.jobs.find((j) => j.id === draft.job) ?? null);
  const on = $derived(TOOLS.filter((t) => t.optional && isOn(draft.features, t.id)).length);
  const optional = TOOLS.filter((t) => t.optional).length;

  let sheet = $state(false);
  let checklist = $state(false);

  async function run(action: () => Promise<unknown>, then?: () => void): Promise<void> {
    try { await action(); then?.(); } catch (error) { ui.say(explain(error), true); }
  }

  function save(): void {
    const batch = draft.sheets?.pages.some(p => !p.job) ?? false;
    const total = draft.sheets?.pages.length ?? 1;
    osk.text(batch ? 'Folder for numbered sheets' : 'Job name', job?.name ?? part?.name ?? 'job', (name) => {
      if (!name.trim()) return;
      run(async () => {
        if (batch) { await api.saveJob(name.trim()); ui.say(`Saved ${total} numbered sheets in ${name.trim()}.`); return; }
        const review = await api.mergeReview(name.trim());
        if (review.conflicts.length) { ui.pendingJobName = name.trim(); ui.modal = 'pending'; return; }
        await api.saveJob(review.name);
        ui.say(`Saved job ${review.name}.`);
      });
    });
  }

  function review(dryRun: boolean): void {
    run(async () => { if (draft.dry_run !== dryRun) await api.compile(dryRun); }, () => { ui.tab = 'run'; });
  }
</script>

<div class="card2" role="button" tabindex="0" onclick={() => (sheet = true)} onkeydown={(e) => { if (e.key === 'Enter') sheet = true; }}>
  <div class="card2-head"><h3>Material</h3><span class="link">Change</span></div>
  {#if recipe}
    {@const art = pictureOf(recipe.name, recipe.photo)}
    <div class="mat-line">{#if art}<img class="swatch lg" src={art} alt="">{:else}<span class="swatch lg" style="background:var(--panel-2)"></span>{/if}<div><div class="big">{recipe.name}</div><div class="muted">{quantity(recipe.thickness_mm, 'mm')} · {recipe.gas} · {laserLabel(recipe.laser)}</div></div></div>
    <div class="stats four">
      <div><b>{displayNumber(recipe.summary.speed, 'mm/s')}</b><span>{unitLabel('mm/s')}</span></div>
      <div><b>{value(recipe.summary.power)}</b><span>% power</span></div>
      <div><b>{displayNumber(recipe.summary.pressure, 'bar')}</b><span>{unitLabel('bar')}</span></div>
      <div><b>{displayNumber(recipe.summary.height, 'mm')}</b><span>{unitLabel('mm')} height</span></div>
    </div>
  {:else}
    <div class="mat-line"><span class="swatch lg" style="background:var(--warn-soft)"></span><div><div class="big warn-text">Choose a material</div><div class="muted">Required before running</div></div></div>
  {/if}
</div>

{#if draft.calibration}<p class="correction-status">Correction off</p>{:else if draft.correction}<p class="correction-status">Matrix correction</p>{/if}

<div class="card2">
  <div class="card2-head"><h3>Machining</h3><span class="muted">{on} of {optional} on</span></div>
  <div class="muted">{#if draft.feature_source}Prefilled from <strong>{draft.feature_source.name}</strong> · {ago(draft.feature_source.at)}{:else}Defaults for this recipe{/if}</div>
</div>

<div class="card2">
  <div class="card2-head"><div><h3>Preflight</h3><span class="muted">{draft.preflight.kind === 'inherit' ? 'Mode defaults' : draft.preflight.kind === 'off' ? 'Checklist off' : `${draft.preflight.steps.length} custom checks`}</span></div><button class="btn btn-ghost" onclick={() => (checklist = true)}>Edit</button></div>
</div>

<div class="stats">
  <div><b>{displayNumber(draft.preview?.length_mm, 'mm', 0)}</b><span>{unitLabel('mm')} cut</span></div>
  <div><b>{draft.compiled ? draft.compiled.pierces.length : '—'}</b><span>pierces</span></div>
  <div><b>{draft.compiled ? seconds(draft.compiled.seconds) : '—'}</b><span>est. time</span></div>
</div>
{#if draft.compiled?.plan.some((pass) => pass.omitted_cooling > 0)}
  <p class="muted">{draft.compiled.plan.reduce((n, pass) => n + pass.omitted_cooling, 0)} cooling points skipped within {quantity(0.2, 'mm')} of endpoints.</p>
{/if}
{#if draft.error}<div class="warn-text" style="font-size:13px">{draft.error}</div>{/if}

<div class="stack">
  <div class="row">
    <button class="btn btn-ghost lg" onclick={save} disabled={!recipe}>{draft.sheets?.pages.some(p => !p.job) ? `Save ${draft.sheets.pages.length} sheets` : 'Save job'}</button>
    <button class="btn btn-ghost lg" onclick={() => review(true)} disabled={!doc.readiness.compile.ok} title={doc.readiness.compile.reason ?? ''}>Dry run</button>
  </div>
  <button class="btn btn-primary xl block" onclick={() => review(false)} disabled={!doc.readiness.compile.ok} title={doc.readiness.compile.reason ?? ''}>Go to Run →</button>
</div>

{#if sheet}<MaterialSheet onclose={() => (sheet = false)} />{/if}
{#if checklist}<PreflightEditor scope="job" onclose={() => (checklist = false)} />{/if}

<style>.correction-status { padding:10px 12px; font-size:11px; color:var(--ink-3); border-left:2px solid var(--accent); }</style>
