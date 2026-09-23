<script lang="ts">
  import ImportParts from '../components/ImportParts.svelte';
  import SimplifyPart from '../components/SimplifyPart.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { ago, explain, laserLabel, recipeLabel, size } from '../lib/format';
  import { boxOf, pathOf, viewBoxFor } from '../lib/svg';
  import { folderPath } from '../lib/folders';
  import { partsOf } from '../lib/job-parts';

  const doc = $derived(server.doc!);
  const library = $derived(doc.library);
  const job = $derived(library.jobs.find((j) => j.id === ui.selected) ?? null);
  /** The parts a saved job cuts; a job whose part is gone is not shown. */
  const jobParts = $derived(job ? partsOf(job, library.parts) : []);
  const part = $derived(job ? null : library.parts.find((p) => p.id === ui.selected) ?? null);
  const item = $derived(job && jobParts.length === job.parts.length ? job : part);
  const folderName = $derived.by(() => {
    const id = item?.folder ?? null;
    return folderPath(library.folders, id).map((folder) => folder.name).join(' / ') || 'All parts';
  });

  async function run(action: () => Promise<unknown>, then?: () => void): Promise<void> {
    try {
      await action();
      then?.();
    } catch (error) {
      ui.say(explain(error), true);
    }
  }

  function open(tab: 'setup' | 'run'): void {
    if (job) run(() => api.openJob(job.id), () => (ui.tab = tab));
    else if (part) run(() => api.openPart(part.id), () => (ui.tab = tab));
  }

  function rename(): void {
    const current = item?.name ?? '';
    osk.text('Rename', current, (name) => {
      if (!name.trim()) return;
      if (job) run(() => api.updateJob(job.id, { name: name.trim() }));
      else if (part) run(() => api.updatePart(part.id, { name: name.trim() }));
    });
  }

  async function remove(): Promise<void> {
    const what = job ? 'job' : 'part';
    const confirmed = await ui.confirm({
      title: `Delete this ${what}?`,
      body: `“${item?.name ?? ''}” leaves the library. Library history can undo it.`,
      confirm: `Delete ${what}`,
      danger: true,
    });
    if (!confirmed) return;
    if (job) run(() => api.removeJob(job.id), () => { ui.selected = null; ui.say('Job deleted.'); });
    else if (part) run(() => api.removePart(part.id), () => { ui.selected = null; ui.say('Part deleted.'); });
  }

  /** Copies the selected saved item without replacing the open draft. */
  function duplicate(): void {
    if (job) {
      run(async () => { const { id } = await api.duplicateJob(job.id); ui.selected = id; }, () => ui.say('Job copied.'));
    } else if (part) {
      run(async () => { const { id } = await api.duplicatePart(part.id); ui.selected = id; }, () => ui.say('Part copied.'));
    }
  }

  function editNotes(): void {
    if (!item) return;
    const id = item.id, savedJob = !!job;
    osk.text('Notes', item.notes, (notes) => run(() => savedJob ? api.updateJob(id, { notes }) : api.updatePart(id, { notes })));
  }

  const outline = $derived(item?.outline ?? []);
  let simplifying = $state(false);
  const thumb = $derived.by(() => { const box = boxOf(outline); return box ? viewBoxFor(box, 100, 70) : '0 0 100 70'; });
</script>

<aside class="panel side">
  {#if !item}
    <h2>Start with a part</h2>
    <p class="muted">Import DXF or SVG, add text, or choose a library part.</p>
    <div class="side-foot"><ImportParts /></div>
  {:else}
    <div class="side-scroll">
    <div class="side-title">
      <div class="thumb" style="width:64px;height:48px;border-radius:8px;background:var(--panel-2);display:grid;place-items:center">
        <svg viewBox={thumb} style="width:80%">{#each outline as line}<path d={pathOf(line)} fill="none" stroke="var(--ink)" stroke-width="2.5" vector-effect="non-scaling-stroke"/>{/each}</svg>
      </div>
      <div><h2>{item.name}</h2><div class="muted">{job ? 'Saved job' : 'Part'}{#if job} · {laserLabel(job.recipe.laser)}{/if} · updated {ago(item.updated)}</div></div>
    </div>
    <dl class="kv">
      <dt>Bounding box</dt><dd>{size(item.bounds)}</dd>
      <dt>Cut paths</dt><dd>{item.contours}</dd>
      <dt>Material</dt><dd>{#if job}{recipeLabel(job.recipe)}{:else}<span class="muted">not set</span>{/if}</dd>
      {#if job && jobParts.length > 1}
        <dt>Parts</dt><dd>{jobParts.map((p) => p.name).join(', ')}</dd>
      {:else}
        <dt>Source</dt><dd class="mono">{(jobParts[0] ?? part)?.file_name}</dd>
      {/if}
      <dt>Folder</dt><dd>{folderName}</dd>
    </dl>
    <div class="item-meta">
      <button class="meta-notes" onclick={editNotes}><span class="meta-label">Notes<span class="edit-label">Edit</span></span><span class:placeholder={!item.notes}>{item.notes || 'Add a note'}</span></button>
    </div>
    <div class="row">
      <button class="btn btn-ghost" onclick={duplicate}>Duplicate</button>
      <button class="btn btn-ghost" onclick={rename}>Rename</button>
      <button class="btn btn-ghost" onclick={remove}>Delete</button>
    </div>
    {#if part}<button class="btn btn-ghost block" onclick={() => (simplifying = true)}>Simplify drawing…</button>{/if}
    </div>
    <div class="side-foot">
      {#if job}
        <button class="btn btn-ghost lg block" onclick={() => open('setup')}>Open in Setup</button>
        <button class="btn btn-primary lg block" onclick={() => open('run')}>▶ Run again</button>
      {:else if part}
        <button class="btn btn-ghost lg block" onclick={() => (ui.partPicks = [part.id])}>Cut with other parts…</button>
        <button class="btn btn-primary lg block" onclick={() => open('setup')}>Set up job →</button>
      {/if}
    </div>
  {/if}
</aside>
{#if simplifying && part}<SimplifyPart {part} onclose={() => (simplifying = false)} />{/if}

<style>
  /* Details scroll on a short screen; the actions at the foot stay in reach. */
  .side-scroll { flex: 1; min-height: 0; overflow-y: auto; overscroll-behavior: contain; display: flex; flex-direction: column; gap: 12px; }
  .item-meta { display: grid; gap: 10px; border-top: 1px solid var(--line); padding-top: 16px; }
  .meta-notes { appearance: none; display: grid; gap: 8px; width: 100%; text-align: left; padding: 14px 16px; font-size: var(--t-sm); border: 1px solid var(--line); border-radius: 12px; background: var(--panel-2); color: var(--ink); cursor: pointer; }
  .meta-notes:hover { border-color: var(--accent); }
  .meta-notes:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .meta-label { display: flex; justify-content: space-between; align-items: center; color: var(--ink-2); font-weight: 600; }
  .edit-label { color: var(--accent-2); font-weight: 600; font-size: var(--t-xs); }
  .placeholder { color: var(--ink-3); }
  .meta-notes { min-height: 100px; }
  .meta-notes > span:last-child { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 110px; overflow: auto; }
</style>
