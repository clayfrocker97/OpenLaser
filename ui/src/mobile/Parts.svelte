<script lang="ts">
  import SkippedFiles from '../components/SkippedFiles.svelte';
  import { access } from '../lib/access.svelte';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { fuzzyScore } from '../lib/search';
  import { recipeLabel, size } from '../lib/format';
  import Preview from './Preview.svelte';
  import Modal from '../components/Modal.svelte';
  import ImportParts from '../components/ImportParts.svelte';
  import SheetLibrary from '../components/SheetLibrary.svelte';
  import EditHistory from '../components/EditHistory.svelte';
  import PartsSide from '../routes/PartsSide.svelte';
  import { api } from '../api/client';
  import { explain } from '../lib/format';
  import { livePicks, togglePick } from '../lib/job-parts';

  let kind = $state<'all' | 'parts' | 'jobs' | 'sheets'>('all');
  let search = $state('');
  let actions = $state(false);
  let history = $state(false);
  let favorites = $state(false);
  let details = $state(false);
  const doc = $derived(server.doc!);
  const library = $derived(doc.library);
  const items = $derived([...library.parts.map(item => ({ item, kind:'parts' as const })), ...library.jobs.map(item => ({ item, kind:'jobs' as const }))]
    .filter(entry => (kind === 'all' || entry.kind === kind) && (!favorites || entry.item.favourite))
    .map(entry => ({ ...entry, score: search.trim() ? fuzzyScore(`${entry.item.name} ${entry.item.notes} ${entry.item.tags.join(' ')} ${'recipe' in entry.item ? recipeLabel(entry.item.recipe) : ''}`, search.trim()) : 1 }))
    .filter(entry => entry.score > 0)
    .sort((a,b) => b.score - a.score || Number(b.item.favourite) - Number(a.item.favourite) || b.item.updated - a.item.updated));
  const blocked = $derived(!access.canControl || !!doc.machine.operation || !server.link);

  // Picking parts to set up as one job, in the order picked.
  const picking = $derived(ui.partPicks !== null);
  const picks = $derived(livePicks(ui.partPicks ?? [], library.parts));
  const inDraft = $derived(doc.draft ? picks.some((id) => doc.draft!.parts.some((p) => p.id === id)) : false);
  let busy = $state(false);
  $effect(() => { if (picking) details = false; });
  function choose(id: string, type: 'parts' | 'jobs'): void {
    if (!picking) { ui.selected = id; details = true; return; }
    if (type === 'jobs') { ui.say('A saved job cuts its own parts. Pick parts to set up a new job.'); return; }
    ui.partPicks = togglePick(picks, id);
  }
  async function finish(add: boolean): Promise<void> {
    if (busy || !picks.length) return;
    busy = true;
    try {
      await (add ? api.addParts(picks) : api.openParts(picks));
      const n = picks.length;
      ui.partPicks = null;
      ui.tab = 'setup';
      ui.say(add ? `Added ${n} part${n === 1 ? '' : 's'} beside the sheet · Undo takes them off` : n > 1 ? `${n} parts side by side · Nest parts fills a sheet` : 'Part opened');
    } catch (error) {
      ui.say(explain(error), true);
    } finally {
      busy = false;
    }
  }
</script>

<div class="phone-page-title"><h1>Parts</h1><div class="phone-title-actions"><button class="btn btn-ghost pick-toggle" aria-pressed={picking} disabled={blocked} onclick={() => (ui.partPicks = picking ? null : [])}>{picking ? 'Cancel' : 'Pick parts'}</button><button class="phone-icon" aria-label="Library actions" onclick={() => actions = true}><i class="ic ic-plus"></i></button></div></div>
<SkippedFiles />
<div class="phone-segments" role="group" aria-label="Library type">
  {#each [['all','All'],['parts','Parts'],['jobs','Jobs']] as [id,label]}<button aria-pressed={kind === id} onclick={() => kind = id as typeof kind}>{label}</button>{/each}
</div>
{#if kind === 'sheets'}
  <div class="phone-collection-heading"><h2>Sheets</h2><button class="btn btn-ghost" onclick={() => kind = 'all'}>All parts</button></div><SheetLibrary />
{:else}
  <label class="phone-search"><i class="ic ic-search"></i><input type="search" aria-label="Search parts and jobs" placeholder="Search parts and jobs" bind:value={search} /></label>
  {#if favorites}<div class="phone-filter-note">Favorites <button onclick={() => favorites = false}>Clear</button></div>{/if}
  <div class="phone-library-list">
    {#each items as { item, kind: type } (item.id)}
      {@const pick = picking && type === 'parts' ? picks.indexOf(item.id) : -1}
      <button class="phone-library-item" class:picked={pick >= 0} class:unpickable={picking && type === 'jobs'} disabled={blocked} aria-pressed={picking && type === 'parts' ? pick >= 0 : undefined} onclick={() => choose(item.id, type)}><Preview outline={item.outline} small /><span><strong>{item.name}</strong><small>{type === 'jobs' && 'recipe' in item ? recipeLabel(item.recipe) : size(item.bounds)}</small></span>{#if picking && type === 'parts'}<span class="pick-mark" class:on={pick >= 0} aria-hidden="true">{pick >= 0 ? pick + 1 : ''}</span>{:else}<i class="ic ic-arrow-right"></i>{/if}</button>
    {:else}<div class="phone-empty"><i class="ic ic-folder"></i><h2>{search ? 'Nothing matches' : 'No parts or jobs yet'}</h2><p>{search ? 'Try a name or material.' : 'Import a drawing to set up your first job.'}</p>{#if !search}<button class="btn btn-primary" onclick={() => actions = true}>Import a drawing</button>{/if}</div>{/each}
  </div>
{/if}

{#if picking}
  <div class="phone-picks-bar">
    <p>{picks.length ? `${picks.length} part${picks.length === 1 ? '' : 's'} picked · laid out in this order` : 'Tap the parts to cut together.'}</p>
    {#if doc.draft}<button class="btn btn-ghost" disabled={busy || blocked || !picks.length || inDraft} onclick={() => finish(true)}>Add to {doc.draft.name}</button>{#if inDraft}<p class="gate-reason">A picked part is already in {doc.draft.name}.</p>{/if}{/if}
    <button class="phone-primary" disabled={busy || blocked || !picks.length} onclick={() => finish(false)}>{picks.length > 1 ? `Set up job with ${picks.length} parts` : 'Set up job'}<i class="ic ic-arrow-right"></i></button>
  </div>
{/if}

{#if actions}<Modal title="Library actions" onclose={() => actions = false}>
  <div class="phone-import-actions"><ImportParts /></div>
  <div class="phone-action-list"><button onclick={() => { actions = false; kind = 'sheets'; }}>Sheets<i class="ic ic-arrow-right"></i></button><button onclick={() => { actions = false; favorites = !favorites; kind = 'all'; }}>Favorites<i class="ic ic-arrow-right"></i></button><button onclick={() => { actions = false; history = true; }}>History<i class="ic ic-arrow-right"></i></button></div>
</Modal>{/if}
{#if history}<EditHistory onclose={() => history = false} />{/if}
{#if details}<Modal title="Part details" onclose={() => details = false}><div class="phone-part-details"><PartsSide /></div></Modal>{/if}

<style>
  .phone-title-actions { display:flex; align-items:center; gap:8px; }
  .pick-toggle { min-height:44px; }
  .pick-toggle[aria-pressed="true"] { border-color:var(--accent); color:var(--accent); }
  .pick-mark { flex:none; width:30px; height:30px; border-radius:50%; border:2px solid var(--ink-3); display:grid; place-items:center; font-weight:700; font-size:var(--t-sm); }
  .pick-mark.on { background:var(--accent); border-color:var(--accent); color:#fff; }
  .phone-library-item.picked { border-color:var(--accent); background:var(--accent-soft); }
  .phone-library-item.unpickable { opacity:.45; }
  .phone-picks-bar { position:sticky; bottom:0; margin-top:12px; padding:12px; display:grid; gap:8px; background:var(--panel); border:1px solid var(--line); border-radius:16px; box-shadow:0 -6px 18px #17230d14; }
  .phone-picks-bar p { margin:0; font-size:var(--t-sm); color:var(--ink-3); }
  .phone-picks-bar .btn { min-height:48px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .phone-picks-bar .phone-primary { background:var(--accent); }
</style>
