<script lang="ts">
  import { quantity } from '../../lib/units.svelte';
  // The library tree: materials under their laser, one opened to its
  // recipes by gas, then thickness. Importing the vendor's process library
  // folder adds every recipe file and the photo of the same name beside it.
  import Modal from '../../components/Modal.svelte';
  import { api } from '../../api/client';
  import { ui } from '../../stores/ui.svelte';
  import { explain, laserLabel } from '../../lib/format';
  import { pictureOf, materialsOf, materialKey, type Material } from '../../lib/materials';
  import type { RecipeView } from '../../api';

  let { recipes, selected, onselect, onadd, selectOnExpand = true }: { recipes: RecipeView[]; selected: RecipeView | null; /** Whether the selection took. */ onselect: (recipe: RecipeView) => boolean; onadd: () => void; selectOnExpand?: boolean } = $props();

  let search = $state('');
  let open = $state<string | null>(null);
  const opened = $derived(open ?? (selected ? materialKey(selected) : null));
  const materials = $derived(materialsOf(recipes).filter((m) => m.name.toLowerCase().includes(search.trim().toLowerCase())));
  const lasers = $derived((['fiber', 'co2'] as const).filter((laser) => materials.some((m) => m.laser === laser)));
  const count = (n: number, what: string) => `${n} ${what}${n === 1 ? '' : 's'}`;
  const thicknessRange = (m: Material) => {
    const sizes = m.recipes.map(r => r.thickness_mm).filter(t => t > 0);
    if (!sizes.length) return 'Thickness not set';
    const min = Math.min(...sizes), max = Math.max(...sizes);
    return min === max ? quantity(min, 'mm') : `${quantity(min, 'mm')} – ${quantity(max, 'mm')}`;
  };
  /** A material's gases, each heading its thicknesses. */
  const gasesOf = (m: Material) => [...new Set(m.recipes.map((r) => r.gas))].sort((a, b) => a.localeCompare(b));
  /** The selected recipe's row stays in view. */
  let tree = $state<HTMLDivElement | null>(null);
  $effect(() => { void selected; tree?.querySelector('.rec.on')?.scrollIntoView({ block: 'nearest' }); });

  function tap(m: Material): void {
    if (opened === m.key) { open = ''; return; }
    if (!selectOnExpand) { open = m.key; return; }
    if (onselect(m.recipes[0]!)) open = m.key;
  }

  // Every recipe file of the folder, then the photo with the same name.
  let importing = $state<{ done: number; total: number } | null>(null);
  let failed = $state<string[]>([]);
  async function importFiles(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const files = [...(input.files ?? [])];
    input.value = '';
    const stem = (f: File) => f.name.replace(/\.[^.]+$/, '');
    const xml = files.filter((f) => /\.xml$/i.test(f.name));
    const photos = new Map(files.filter((f) => /\.png$/i.test(f.name)).map((f) => [stem(f), f]));
    if (!xml.length) { ui.say('No recipe files (.xml) in the folder.', true); return; }
    importing = { done: 0, total: xml.length };
    const tally = { added: 0, known: 0, failed: [] as string[] };
    for (const file of xml) {
      try {
        const { id, existing } = await api.importRecipe(file.name, await file.arrayBuffer());
        if (existing) tally.known += 1; else tally.added += 1;
        const photo = photos.get(stem(file));
        if (photo) await api.setPhoto(id, await photo.arrayBuffer());
      } catch (error) {
        tally.failed.push(`${file.name}: ${explain(error)}`);
      }
      importing.done += 1;
    }
    importing = null;
    failed = tally.failed;
    ui.say(`${count(tally.added, 'recipe')} added, ${tally.known} already in the library${tally.failed.length ? `, ${tally.failed.length} failed` : ''}.`);
  }
</script>

<aside class="panel library">
  <div class="library-head"><h2>Materials</h2><button class="btn btn-primary icon-only" onclick={onadd} aria-label="New recipe"><i class="ic ic-plus"></i></button></div>
  <div class="search"><i class="ic ic-search"></i><input placeholder="Find a material…" bind:value={search}></div>
  <div class="tree" bind:this={tree}>
    {#each lasers as laser (laser)}
      {#if lasers.length > 1}<h3>{laserLabel(laser)}</h3>{/if}
      {#each materials.filter((m) => m.laser === laser) as m (m.key)}
        {@const art = pictureOf(m.name, m.photo)}
        <button class="mat" class:open={opened === m.key} aria-expanded={opened === m.key} onclick={() => tap(m)}>
          {#if art}<img class="art" src={art} alt="" loading="lazy">{:else}<span class="art blank"><i class="ic ic-layers"></i></span>{/if}
          <span class="mat-info"><span class="name">{m.name}</span><span class="mat-meta">{count(m.recipes.length, 'recipe')}</span><span class="mat-meta">{thicknessRange(m)}</span></span><i class="ic ic-chev-right mat-chevron"></i>
        </button>
        {#if opened === m.key}
          {#each gasesOf(m) as gas (gas)}
            <span class="gas">{gas}</span>
            {#each m.recipes.filter((r) => r.gas === gas) as r (r.id)}
              <button class="rec" class:on={r.id === selected?.id} aria-pressed={r.id === selected?.id} onclick={() => onselect(r)}><strong>{r.thickness_mm > 0 ? quantity(r.thickness_mm, 'mm') : 'No thickness'}</strong>{#if r.attributes['OpenLaserNozzleDiameter']}<small>Ø {quantity(Number(r.attributes['OpenLaserNozzleDiameter']), 'mm')}{r.attributes['OpenLaserNozzleType'] === 'double' ? ' · Double' : r.attributes['OpenLaserNozzleType'] === 'single' ? ' · Single' : ''}</small>{/if}</button>
            {/each}
          {/each}
        {/if}
      {/each}
    {:else}
      <p class="muted">{search ? 'No material matches.' : 'No recipes yet.'}</p>
    {/each}
  </div>
  <div class="library-foot">
    <label class="btn btn-ghost block">{importing ? `Importing ${importing.done} of ${importing.total}…` : 'Import library'}<input type="file" multiple webkitdirectory accept=".xml,.png" hidden onchange={importFiles} disabled={!!importing}></label>
    <span class="muted">{count(materials.length, 'material')} · {count(recipes.length, 'recipe')}</span>
  </div>
</aside>

{#if failed.length}
  <Modal title="Files that did not import" onclose={() => (failed = [])}>
    <div class="import-log">{failed.join('\n')}</div>
    <button class="btn btn-primary block" onclick={() => (failed = [])}>OK</button>
  </Modal>
{/if}
