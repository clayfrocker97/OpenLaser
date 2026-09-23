<script lang="ts">
  import { quantity } from '../../lib/units.svelte';
  // A new recipe, full screen: the laser, then the material from the
  // cards or a custom name; under them the thickness, the gas for fiber,
  // and where the values start from: a recipe of the same material, or
  // a layer bank of the machine files.
  import { untrack } from 'svelte';
  import { api } from '../../api/client';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { explain, plural } from '../../lib/format';
  import { GAS } from '../../lib/recipe';
  import { summaryLine } from '../../lib/summary';
  import { CARDS, cardUrl, type MaterialCard } from '../../lib/materials';
  import type { LaserMode, Values } from '../../api';

  let { onclose, onadded }: { onclose: () => void; onadded: (id: string) => void } = $props();
  const doc = $derived(server.doc!);
  const recipes = $derived(doc.library.recipes);
  const mm = (t: number) => (t > 0 ? quantity(t, 'mm') : 'thickness not set');

  let draft = $state({
    laser: 'fiber' as LaserMode,
    name: '',
    thickness_mm: 1,
    gas: null as number | null,
    values: null as Values | null,
  });
  let search = $state('');
  let category = $state('');
  // Metals for the fiber laser, everything else for CO₂.
  const cards = $derived(CARDS.filter((c) => (c.category === 'Metals') === (draft.laser === 'fiber')));
  const categories = $derived([...new Set(cards.map((c) => c.category))]);
  const matching = $derived(cards.filter((c) =>
    (!category || c.category === category)
    && (!search.trim() || c.name.toLowerCase().includes(search.trim().toLowerCase()))));
  const custom = $derived(!!draft.name && !CARDS.some((c) => c.name === draft.name));
  const gases = $derived([...new Set([...doc.files.gases, ...(draft.gas === null ? [] : [draft.gas])])].sort((a, b) => a - b));

  // As many cards as fit, the custom tile first; the rest on further pages.
  let grid = $state<HTMLDivElement | null>(null);
  let per = $state(12);
  let page = $state(0);
  $effect(() => {
    if (!grid) return;
    const measure = () => {
      const rect = grid!.getBoundingClientRect();
      const tile = parseFloat(getComputedStyle(grid!).getPropertyValue('--tile')) + 12;
      const cols = Math.max(1, Math.floor((rect.width - 32 + 12) / tile));
      per = cols * Math.max(1, Math.floor((rect.height - 20 + 12) / tile));
      grid!.style.gridTemplateColumns = `repeat(${cols}, 1fr)`;
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(grid);
    return () => observer.disconnect();
  });
  const tiles = $derived<Array<MaterialCard | null>>([null, ...matching]);
  const pages = $derived(Math.max(1, Math.ceil(tiles.length / per)));
  $effect(() => { if (page > pages - 1) page = pages - 1; });
  const paged = $derived(tiles.slice(page * per, page * per + per));

  /** Where the values can start from: recipes of the material, then the machine's banks. */
  type Source = { values: Values; label: string; meta: string; gas: string | null };
  const sources = $derived.by((): Source[] => {
    const own = recipes
      .filter((r) => r.laser === draft.laser && r.name === draft.name)
      .sort((a, b) => a.thickness_mm - b.thickness_mm);
    const list: Source[] = own.map((r) => ({
      values: { recipe: r.id },
      label: `${r.name} · ${mm(r.thickness_mm)} · ${r.gas}`,
      meta: summaryLine(r),
      gas: r.attributes['CutGasType'] ?? null,
    }));
    for (const b of doc.files.banks.filter((b) => b.laser === draft.laser)) {
      list.push({
        values: { bank: b.bank },
        label: `Machine bank ${b.bank}${b.name ? ` · ${b.name}` : ''}${b.disabled ? ' · disabled' : ''}`,
        meta: summaryLine({ laser: b.laser, gas: b.summary.gas === null ? '' : GAS[Number(b.summary.gas)] ?? '', summary: b.summary }),
        gas: b.summary.gas ?? null,
      });
    }
    return list;
  });
  const same = (a: Values | null, b: Values) => JSON.stringify(a) === JSON.stringify(b);
  // The first source is taken until the operator picks one; its gas comes along.
  $effect(() => {
    const first = sources[0];
    untrack(() => {
      if (!first) draft.values = null;
      else if (!draft.values || !sources.some((s) => same(draft.values, s.values))) choose(first);
    });
  });
  function choose(source: Source): void {
    draft.values = source.values;
    if (source.gas !== null) draft.gas = Number(source.gas);
  }
  function setLaser(laser: LaserMode): void {
    draft.laser = laser;
    category = '';
    if (!custom && !cards.some((c) => c.name === draft.name)) draft.name = '';
  }
  function nameCustom(): void {
    osk.text('Material', custom ? draft.name : '', (v) => { draft.name = v.trim(); });
  }
  function editThickness(): void {
    osk.number('Thickness', draft.thickness_mm, 'mm', (v) => { if (v >= 0) draft.thickness_mm = v; });
  }
  function toggleCategory(c: string): void {
    category = category === c ? '' : c;
  }
  const addLabel = $derived(draft.name ? `Add ${draft.name} ${mm(draft.thickness_mm)}` : 'Choose a material');
  async function add(): Promise<void> {
    const name = draft.name.trim();
    if (!name || !draft.values) return;
    try {
      const { id } = await api.addRecipe({
        name,
        laser: draft.laser,
        thickness_mm: draft.thickness_mm,
        values: draft.values,
        gas: draft.laser === 'fiber' ? draft.gas : null,
      });
      ui.selectedRecipe = id;
      ui.say(`${name} ${mm(draft.thickness_mm)} added.`);
      onadded(id);
    } catch (error) {
      ui.say(explain(error), true);
    }
  }
</script>

<section class="panel main">
  <div class="panel-head">
    <div class="minw0">
      <h3>Material library · New recipe</h3>
      <h1>{draft.name || 'Choose a material'}</h1>
      <p class="muted">{draft.laser === 'fiber' ? 'Metals, for the fiber laser.' : 'Everything else, for the CO₂ laser.'} Pick a card or name your own.</p>
    </div>
    <div class="actions">
      <div class="seg">
        {#each [['fiber', 'Fiber'], ['co2', 'CO₂']] as [k, label] (k)}
          <button class:on={draft.laser === k} onclick={() => setLaser(k as LaserMode)}>{label}</button>
        {/each}
      </div>
      <button class="btn btn-ghost icon-only" onclick={onclose} aria-label="Close"><i class="ic ic-x"></i></button>
    </div>
  </div>
  <div class="toolbar">
    {#if categories.length > 1}
      <div class="chips cats">
        <button class="chip" class:on={!category} onclick={() => (category = '')}>All</button>
        {#each categories as c (c)}
          <button class="chip" class:on={category === c} onclick={() => toggleCategory(c)}>{c}</button>
        {/each}
      </div>
    {/if}
    <div class="search"><i class="ic ic-search"></i><input placeholder="Find a material…" bind:value={search}></div>
  </div>
  <div class="grid pick-grid" bind:this={grid}>
    {#each paged as c (c?.id ?? 'custom')}
      {#if c === null}
        <button class="pick" class:on={custom} onclick={nameCustom}>
          <span class="art blank">Aa</span>
          <span>{custom ? draft.name : 'Custom name…'}</span>
        </button>
      {:else}
        <button class="pick" class:on={draft.name === c.name} onclick={() => (draft.name = c.name)}>
          <img class="art" src={cardUrl(c.id)} alt="" loading="lazy">
          <span>{c.name}</span>
        </button>
      {/if}
    {/each}
  </div>
  <div class="new-foot">
    <div class="field">
      <h3>Thickness</h3>
      <button class="btn btn-ghost" data-numpad onclick={editThickness}>{mm(draft.thickness_mm)}</button>
    </div>
    {#if draft.laser === 'fiber'}
      <div class="field">
        <h3>Assist gas</h3>
        <div class="chips">
          {#each gases as selector (selector)}
            <button class="chip" class:on={draft.gas === selector} onclick={() => (draft.gas = selector)}>
              {GAS[selector] ?? `Selection ${selector}`}
            </button>
          {/each}
        </div>
      </div>
    {/if}
    <div class="field grow">
      <h3>Start from</h3>
      <div class="from-row">
        {#each sources as source (source.label)}
          <button class="from" class:on={same(draft.values, source.values)} onclick={() => choose(source)}>
            <span>{source.label}</span>
            <small>{source.meta}</small>
          </button>
        {:else}
          <p class="muted">{doc.files.error ?? 'The machine files hold no banks for this laser; import the machine backup on the Settings page.'}</p>
        {/each}
      </div>
    </div>
  </div>
  <div class="panel-foot">
    <span class="muted">{plural(matching.length, 'material')}</span>
    <div class="pager">
      <button onclick={() => (page = Math.max(0, page - 1))} disabled={page === 0} aria-label="Previous page"><i class="ic ic-chev-left"></i></button>
      <span>{page + 1} / {pages}</span>
      <button onclick={() => (page = Math.min(pages - 1, page + 1))} disabled={page >= pages - 1} aria-label="Next page"><i class="ic ic-chev-right"></i></button>
    </div>
    <button class="btn btn-primary" onclick={add} disabled={!draft.name || !draft.values}>{addLabel}</button>
  </div>
</section>
