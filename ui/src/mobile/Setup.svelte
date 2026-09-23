<script lang="ts">
  import { api } from '../api/client';
  import { access } from '../lib/access.svelte';
  import { server } from '../stores/server.svelte';
  import { ui, type FeatureId } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain, recipeLabel, size } from '../lib/format';
  import { diagnosticText } from '../lib/units.svelte';
  import { TOOLS, isOn, stateOf } from '../lib/features';
  import Preview from './Preview.svelte';
  import Modal from '../components/Modal.svelte';
  import PreflightEditor from '../components/PreflightEditor.svelte';
  import { onlyPart } from '../lib/job-parts';
  import AddParts from '../components/AddParts.svelte';

  let { edit }: { edit: (tool:FeatureId | 'copy' | 'nest' | 'layout') => void } = $props();
  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft);
  const part = $derived(onlyPart(draft, doc.library.parts));
  const job = $derived(doc.library.jobs.find(j => j.id === draft?.job));
  const outline = $derived(draft?.preview?.contours.flatMap(c => c.paths.filter(p => p.kind === 'cut').map(p => p.points)) ?? part?.outline ?? []);
  const active = $derived(draft ? TOOLS.filter(t => t.optional && isOn(draft.features,t.id)).length : 0);
  let busy = $state(false);
  let material = $state(false);
  let search = $state('');
  let tools = $state(false);
  let preflight = $state(false);
  let adding = $state(false);
  const blocked = $derived(!access.canControl || busy || !!doc.machine.operation || !server.link);
  const recipes = $derived(doc.library.recipes.filter(r => recipeLabel(r).toLocaleLowerCase().includes(search.toLocaleLowerCase())).sort((a,b) => Number(b.laser === doc.mode) - Number(a.laser === doc.mode) || Number(b.favourite) - Number(a.favourite) || a.name.localeCompare(b.name)));

  async function perform(action: () => Promise<unknown>): Promise<void> {
    if (blocked) return;
    busy = true;
    try { await action(); } catch (error) { ui.say(explain(error),true); }
    finally { busy = false; }
  }
  function review(dryRun:boolean): void { void perform(async () => { if (draft?.dry_run !== dryRun) await api.compile(dryRun); ui.tab = 'run'; }); }
  function save(): void {
    if (!draft) return;
    osk.text(draft.sheets?.pages.some(p => !p.job) ? 'Folder for numbered sheets' : 'Job name',job?.name ?? (draft.name || 'job'),name => {
      if (!name.trim()) return;
      void perform(async () => {
        if (draft.sheets?.pages.some(p => !p.job)) { await api.saveJob(name.trim()); ui.say('Saved numbered sheets.'); return; }
        const review = await api.mergeReview(name.trim());
        if (review.conflicts.length) { ui.pendingJobName = name.trim(); ui.modal = 'pending'; return; }
        await api.saveJob(review.name); ui.say(`Saved job ${review.name}.`);
      });
    });
  }
</script>

<div class="phone-run phone-setup-home">
  <div class="phone-run-scroll">
    <div class="phone-page-title"><h1>Setup</h1></div>
    {#if draft}
      <section class="phone-job-card">
        <div class="phone-job-heading"><div><h2>{job?.name ?? (draft.name || 'Current job')}</h2></div></div>
        <Preview {outline} />
        <div class="phone-job-spec"><span>{size(draft.preview?.bounds ?? part?.bounds ?? null)}</span><button class="phone-text-action" disabled={blocked} onclick={() => edit('layout')}>Edit layout</button></div>
      </section>
      <button class="phone-material" disabled={blocked} onclick={() => material = true}><i class="ic ic-layers"></i><span><small>Material</small><strong>{draft.recipe ? recipeLabel(draft.recipe) : 'Choose a material'}</strong></span><i class="ic ic-arrow-right"></i></button>
      <div class="phone-action-list phone-setup-options">
        <button disabled={blocked} onclick={() => adding = true}><span>Parts<small>{draft.parts.length === 1 ? '1 part · Add more to cut them together' : `${draft.parts.length} parts side by side · Add more`}</small></span><i class="ic ic-plus"></i></button>
        <button disabled={blocked} onclick={() => tools = true}><span>Machining<small>{active ? `${active} tools on` : 'Default settings'}</small></span><i class="ic ic-arrow-right"></i></button>
        <button disabled={blocked} onclick={() => preflight = true}><span>Preflight<small>{draft.preflight.kind === 'inherit' ? 'Mode defaults' : draft.preflight.kind === 'off' ? 'Checklist off' : `${draft.preflight.steps.length} checks`}</small></span><i class="ic ic-arrow-right"></i></button>
      </div>
    {:else}<div class="phone-empty"><i class="ic ic-folder"></i><h2>No part open</h2><p>Choose a part or saved job to set up.</p><button class="phone-primary" onclick={() => ui.tab = 'parts'}>Go to Parts</button></div>{/if}
  </div>
  {#if draft}<div class="phone-run-footer">
    <div class="phone-secondary-actions"><button class="btn btn-ghost" disabled={blocked || !draft.recipe} onclick={save}>{draft.sheets?.pages.some(p => !p.job) ? `Save ${draft.sheets.pages.length} sheets` : 'Save job'}</button><button class="btn btn-ghost" disabled={blocked || !doc.readiness.compile.ok} onclick={() => review(true)}>Dry run</button></div>
    <button class="phone-primary" disabled={blocked || !doc.readiness.compile.ok} onclick={() => review(false)}>{busy ? 'Preparing…' : 'Go to Run'}<i class="ic ic-arrow-right"></i></button>
    {#if !doc.readiness.compile.ok}<p class="phone-readiness">{diagnosticText(doc.readiness.compile.reason ?? '')}</p>{/if}
  </div>{/if}
</div>

{#if material}<Modal title="Material" onclose={() => material = false}><div class="phone-material-picker"><input type="search" aria-label="Search materials" placeholder="Search materials" bind:value={search} />{#each recipes as recipe}<button disabled={busy} onclick={() => perform(async () => { await api.setRecipe(recipe.id); material = false; })}><strong>{recipeLabel(recipe)}</strong><small>{recipe.laser === 'co2' ? 'CO₂' : 'Fiber'}</small></button>{:else}<p>No matching materials.</p>{/each}</div></Modal>{/if}
{#if tools}<Modal title="Machining" onclose={() => tools = false}><div class="phone-action-list">{#each TOOLS as tool}<button onclick={() => { tools = false; edit(tool.id as FeatureId); }}><span>{tool.name}<small>{draft ? stateOf(draft.features,tool.id) : ''}</small></span><i class="ic ic-arrow-right"></i></button>{/each}<button onclick={() => { tools = false; edit('nest'); }}>Nest parts<i class="ic ic-arrow-right"></i></button><button onclick={() => { tools = false; edit('copy'); }}>Copy machining from job<i class="ic ic-arrow-right"></i></button></div></Modal>{/if}
{#if preflight}<PreflightEditor scope="job" onclose={() => preflight = false} />{/if}
{#if adding}<AddParts onclose={() => adding = false} />{/if}
