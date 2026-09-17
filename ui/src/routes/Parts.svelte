<script lang="ts">
  import SheetLibrary from '../components/SheetLibrary.svelte';
  import ImportParts from '../components/ImportParts.svelte';
  import EditHistory from '../components/EditHistory.svelte';
  import PartsSide from './PartsSide.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { fuzzyScore } from '../lib/search';
  import { folderPath as pathOfFolder } from '../lib/folders';
  import { explain, laserLabel, recipeLabel, size } from '../lib/format';
  import { boxOf, pathOf, viewBoxFor } from '../lib/svg';
  import type { Folder, JobView, PartView } from '../api';

  const doc = $derived(server.doc!);
  const library = $derived(doc.library);

  /** A card: a part or a saved job. */
  type Card = { id: string; kind: 'part' | 'job'; name: string; search: string; folder: string | null; laser: 'fiber' | 'co2' | null; material: string; size: string; contours: number; favourite: boolean; updated: number; outline: number[][][]; part: PartView };

  let kindFilter = $state<'all' | 'part' | 'job' | 'fav' | 'sheet'>('all');
  let laserFilter = $state<'all' | 'fiber' | 'co2'>('all');
  let page = $state(0);
  let historyOpen = $state(false);

  const cards = $derived.by((): Card[] => {
    const parts = library.parts.map((p): Card => ({ id: p.id, kind: 'part', name: p.name, search: `${p.tags.join(' ')} ${p.notes}`, folder: p.folder, laser: null, material: '', size: size(p.bounds), contours: p.contours, favourite: p.favourite, updated: p.updated, outline: p.outline, part: p }));
    const jobs = library.jobs.flatMap((j: JobView): Card[] => {
      const part = library.parts.find((p) => p.id === j.part);
      if (!part) return [];
      return [{ id: j.id, kind: 'job', name: j.name, search: `${j.tags.join(' ')} ${j.notes}`, folder: j.folder, laser: j.recipe.laser, material: recipeLabel(j.recipe), size: size(j.bounds), contours: j.contours, favourite: j.favourite, updated: j.updated, outline: j.outline, part }];
    });
    return [...parts, ...jobs];
  });

  const folderPath = (id: string | null) => pathOfFolder(library.folders, id);

  const q = $derived(ui.search.trim());
  const visible = $derived.by(() => {
    let items = cards.filter((c) => (kindFilter === 'all' || (kindFilter === 'fav' ? c.favourite : c.kind === kindFilter)) && (laserFilter === 'all' || c.laser === laserFilter || (c.laser === null && kindFilter !== 'job')));
    if (q) {
      return items.map((c) => [fuzzyScore(`${c.name} ${c.search} ${c.material} ${c.size} ${folderPath(c.folder).map((f) => f.name).join(' ')}`, q), c] as const).filter((x) => x[0] > 0).sort((a, b) => b[0] - a[0]).map((x) => x[1]);
    }
    return items.filter((c) => (c.folder ?? null) === ui.folder);
  });
  const folders = $derived(q ? [] : library.folders.filter((f) => (f.parent ?? null) === ui.folder).sort((a, b) => Number(b.favourite) - Number(a.favourite)));

  // Cards never scroll: as many as fit, the rest on further pages.
  let grid = $state<HTMLDivElement | null>(null);
  let per = $state(8);
  $effect(() => {
    if (!grid) return;
    const measure = () => {
      const rect = grid!.getBoundingClientRect();
      const cols = Math.max(1, Math.floor((rect.width - 44 + 12) / (210 + 12)));
      const rows = Math.max(1, Math.floor((rect.height - 24 + 12) / (172 + 12)));
      per = cols * rows;
      grid!.style.gridTemplateColumns = `repeat(${cols}, minmax(0, 1fr))`;
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(grid);
    return () => observer.disconnect();
  });
  const total = $derived(folders.length + visible.length);
  const pages = $derived(Math.max(1, Math.ceil(total / per)));
  $effect(() => { if (page > pages - 1) page = pages - 1; });
  const shown = $derived.by(() => {
    const all: Array<{ folder: Folder } | { card: Card }> = [...folders.map((folder) => ({ folder })), ...visible.map((card) => ({ card }))];
    return all.slice(page * per, page * per + per);
  });

  const thumb = (outline: number[][][]) => { const box = boxOf(outline); return box ? viewBoxFor(box, 100, 70) : '0 0 100 70'; };

  async function star(card: Card): Promise<void> {
    try {
      await (card.kind === 'job' ? api.updateJob(card.id, { favourite: !card.favourite }) : api.updatePart(card.id, { favourite: !card.favourite }));
    } catch (error) {
      ui.say(explain(error), true);
    }
  }

  function newFolder(): void {
    osk.text('New folder name', '', async (name) => {
      if (!name.trim()) return;
      try { await api.addFolder(name.trim(), ui.folder); } catch (error) { ui.say(explain(error), true); }
    });
  }
</script>

<section class="panel main" class:sheet-library-page={kindFilter === 'sheet'}>
  <div class="toolbar head-row parts-head">
    <h1>Parts</h1>
    <div class="seg">
      {#each [['all', 'All'], ['part', 'Parts'], ['job', 'Jobs'], ['sheet', 'Sheets']] as [k, label]}<button class:on={kindFilter === k} onclick={() => (kindFilter = k as typeof kindFilter)}>{label}</button>{/each}
      <button class:on={kindFilter === 'fav'} onclick={() => (kindFilter = 'fav')} title="Favorites"><i class="ic ic-star"></i></button>
    </div>
    {#if kindFilter !== 'sheet'}<div class="seg">
      {#each [['all', 'All'], ['fiber', 'Fiber'], ['co2', 'CO₂']] as [k, label]}<button class:on={laserFilter === k} onclick={() => (laserFilter = k as typeof laserFilter)}>{label}</button>{/each}
    </div>{/if}
  </div>
  {#if kindFilter !== 'sheet'}<div class="toolbar parts-actions">
    <div class="search"><i class="ic ic-search"></i><input placeholder="Search everything: name, material, size, folder…" bind:value={ui.search} onkeydown={(e) => { if (e.key === 'Escape') ui.search = ''; }}></div>
    <button class="btn btn-ghost icon-btn" onclick={newFolder} title="New folder"><i class="ic ic-folder-plus"></i><span>Folder</span></button>
    <ImportParts />
  </div>{/if}

  {#if kindFilter === 'sheet'}<SheetLibrary />{:else}
  <div class="crumbs">
    {#if q}
      <span class="muted">Results everywhere for</span> <strong>“{q}”</strong>
    {:else}
      <button class="crumb-btn" onclick={() => (ui.folder = null)}>All parts</button>
      {#each folderPath(ui.folder) as f}<i class="ic ic-chev-right sm"></i><button class="crumb-btn" onclick={() => (ui.folder = f.id)}>{f.name}</button>{/each}
    {/if}
  </div>

  <div class="grid parts-grid" bind:this={grid}>
    {#each shown as item}
      {#if 'folder' in item}
        {@const n = cards.filter((c) => folderPath(c.folder).some((f) => f.id === item.folder.id)).length}
        <div class="card folder" role="button" tabindex="0" onclick={() => { ui.folder = item.folder.id; ui.search = ''; }} onkeydown={(e) => { if (e.key === 'Enter') { ui.folder = item.folder.id; } }}>
          <button class="fav" class:on={item.folder.favourite} aria-label="Star" onclick={(e) => { e.stopPropagation(); api.updateFolder(item.folder.id, { favourite: !item.folder.favourite }).catch((error) => ui.say(explain(error), true)); }}><i class="ic {item.folder.favourite ? 'ic-star-fill' : 'ic-star'}"></i></button>
          <div class="thumb"><i class="ic ic-folder lg"></i></div>
          <div class="title">{item.folder.name}</div>
          <div class="sub">{n} item{n === 1 ? '' : 's'}</div>
        </div>
      {:else}
        {@const c = item.card}
        <div class="card" class:selected={c.id === ui.selected} role="button" tabindex="0" onclick={() => (ui.selected = c.id)} onkeydown={(e) => { if (e.key === 'Enter') ui.selected = c.id; }}>
          <button class="fav" class:on={c.favourite} aria-label="Star" onclick={(e) => { e.stopPropagation(); star(c); }}><i class="ic {c.favourite ? 'ic-star-fill' : 'ic-star'}"></i></button>
          <div class="thumb"><svg viewBox={thumb(c.outline)}>{#each c.outline as line}<path d={pathOf(line)} fill="none" stroke="var(--ink)" stroke-width="2" stroke-linejoin="round" vector-effect="non-scaling-stroke"/>{/each}</svg></div>
          <div class="title">{c.name}</div>
          <div class="sub">{#if q && c.folder}<i class="ic ic-folder sm"></i>{folderPath(c.folder).map((f) => f.name).join(' / ')} · {/if}{c.material ? `${c.material} · ` : ''}{c.size}</div>
          <div class="tags">
            {#if c.laser}<span class="tag {c.laser}">{laserLabel(c.laser)}</span>{/if}
            <span class="tag">{c.kind === 'job' ? 'Job' : 'Part'}</span>
            <span class="tag">{c.contours} paths</span>
          </div>
        </div>
      {/if}
    {:else}
      <div class="empty"><h2>{q ? 'Nothing matches' : 'Empty folder'}</h2>{q ? 'Try a shorter word, a material, a size, or a folder name.' : 'Import DXF or SVG, add text, or move parts here.'}</div>
    {/each}
  </div>

  <div class="panel-foot">
    <button class="btn btn-ghost" onclick={() => (historyOpen = true)}>History</button>
    <span class="muted">{q ? `${visible.length} result${visible.length === 1 ? '' : 's'}` : `${total} items`}</span>
    <div class="pager"><button onclick={() => (page = Math.max(0, page - 1))} disabled={page === 0} aria-label="Previous page"><i class="ic ic-chev-left"></i></button><span>{page + 1} / {pages}</span><button onclick={() => (page = Math.min(pages - 1, page + 1))} disabled={page >= pages - 1} aria-label="Next page"><i class="ic ic-chev-right"></i></button></div>
  </div>
{/if}
</section>

{#if kindFilter !== 'sheet'}<PartsSide />{/if}
{#if historyOpen}<EditHistory onclose={() => (historyOpen = false)} />{/if}

<style>
  .sheet-library-page { grid-column:1 / -1; }
  .toolbar.parts-head { flex-wrap:wrap; flex-shrink:0; }
  .parts-actions { flex-shrink:0; padding:12px 16px; border-bottom:1px solid var(--line); gap:8px; }
  .parts-actions .search { min-width:120px; flex:1; }
  .parts-actions input { min-width:0; width:0; }
</style>
