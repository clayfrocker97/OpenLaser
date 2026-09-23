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
</script>

<div class="phone-page-title"><h1>Parts</h1><button class="phone-icon" aria-label="Library actions" onclick={() => actions = true}><i class="ic ic-plus"></i></button></div>
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
      <button class="phone-library-item" disabled={blocked} onclick={() => { ui.selected = item.id; details = true; }}><Preview outline={item.outline} small /><span><strong>{item.name}</strong><small>{type === 'jobs' && 'recipe' in item ? recipeLabel(item.recipe) : size(item.bounds)}</small></span><i class="ic ic-arrow-right"></i></button>
    {:else}<div class="phone-empty"><i class="ic ic-folder"></i><h2>{search ? 'Nothing matches' : 'No parts or jobs yet'}</h2><p>{search ? 'Try a name or material.' : 'Import a drawing to set up your first job.'}</p>{#if !search}<button class="btn btn-primary" onclick={() => actions = true}>Import a drawing</button>{/if}</div>{/each}
  </div>
{/if}

{#if actions}<Modal title="Library actions" onclose={() => actions = false}>
  <div class="phone-import-actions"><ImportParts /></div>
  <div class="phone-action-list"><button onclick={() => { actions = false; kind = 'sheets'; }}>Sheets<i class="ic ic-arrow-right"></i></button><button onclick={() => { actions = false; favorites = !favorites; kind = 'all'; }}>Favorites<i class="ic ic-arrow-right"></i></button><button onclick={() => { actions = false; history = true; }}>History<i class="ic ic-arrow-right"></i></button></div>
</Modal>{/if}
{#if history}<EditHistory onclose={() => history = false} />{/if}
{#if details}<Modal title="Part details" onclose={() => details = false}><div class="phone-part-details"><PartsSide /></div></Modal>{/if}
