<script lang="ts">
  import { quantity } from '../../lib/units.svelte';
  // The library tree: materials under their laser, one opened to its
  // recipes by gas, then thickness. Importing the vendor's process library
  // folder adds every recipe file and the photo of the same name beside it.
  import MaterialSummary from '../../components/MaterialSummary.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { explain, laserLabel, plural } from '../../lib/format';
  import { pictureOf, materialsOf, materialKey, type Material } from '../../lib/materials';
  import { droppedFiles, picked, type Picked } from '../../lib/recipe-import';
  import ImportRecipes from './ImportRecipes.svelte';
  import type { RecipeView } from '../../api';

  let { recipes, selected, onselect, onadd, selectOnExpand = true }: {
    recipes: RecipeView[];
    selected: RecipeView | null;
    /** Whether the selection took. */
    onselect: (recipe: RecipeView) => boolean;
    onadd: () => void;
    selectOnExpand?: boolean;
  } = $props();

  let search = $state('');
  let open = $state<string | null>(null);
  const opened = $derived(open ?? (selected ? materialKey(selected) : null));
  const materials = $derived(materialsOf(recipes).filter((m) => m.name.toLowerCase().includes(search.trim().toLowerCase())));
  const lasers = $derived((['fiber', 'co2'] as const).filter((laser) => materials.some((m) => m.laser === laser)));
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

  // Import: recipe files, a whole folder, or files and folders dropped on
  // the library. Each recipe file brings the photo of the same name beside
  // it, and everything is reviewed before it is saved.
  let importing = $state<Picked[] | null>(null);
  let dragging = $state(false);
  function chosen(event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    const files = picked(input.files ?? []);
    input.value = '';
    if (files.length) importing = files;
  }
  function drop(event: DragEvent): void {
    event.preventDefault();
    dragging = false;
    if (!event.dataTransfer) return;
    droppedFiles(event.dataTransfer).then((files) => { if (files.length) importing = files; }).catch((error) => ui.say(explain(error), true));
  }
  const carriesFiles = (event: DragEvent) => event.dataTransfer?.types.includes('Files') ?? false;
</script>

<aside class="panel library" class:dragging aria-label="Material library"
  ondragover={(e) => { if (carriesFiles(e)) { e.preventDefault(); dragging = true; } }}
  ondragleave={(e) => { if (e.currentTarget === e.target) dragging = false; }}
  ondrop={drop}>
  <div class="library-head"><h2>Materials</h2><button class="btn btn-primary icon-only" onclick={onadd} aria-label="New recipe"><i class="ic ic-plus"></i></button></div>
  <div class="search"><i class="ic ic-search"></i><input placeholder="Find a material…" bind:value={search}></div>
  <div class="tree" bind:this={tree}>
    {#each lasers as laser (laser)}
      {#if lasers.length > 1}<h3>{laserLabel(laser)}</h3>{/if}
      {#each materials.filter((m) => m.laser === laser) as m (m.key)}
        {@const art = pictureOf(m.name, m.photo)}
        <button class="mat" class:open={opened === m.key} aria-expanded={opened === m.key} onclick={() => tap(m)}>
          {#if art}<img class="art" src={art} alt="" loading="lazy">{:else}<span class="art blank"><i class="ic ic-layers"></i></span>{/if}
          <span class="mat-info"><span class="name">{m.name}</span><span class="mat-meta">{plural(m.recipes.length, 'recipe')}</span><span class="mat-meta">{thicknessRange(m)}</span></span><i class="ic ic-chev-right mat-chevron"></i>
        </button>
        {#if opened === m.key}
          {#each gasesOf(m) as gas (gas)}
            <span class="gas">{gas}</span>
            {#each m.recipes.filter((r) => r.gas === gas) as r (r.id)}
              <button
                class="rec" class:on={r.id === selected?.id} aria-pressed={r.id === selected?.id} onclick={() => onselect(r)}
              ><strong>{r.thickness_mm > 0 ? quantity(r.thickness_mm, 'mm') : 'No thickness'}</strong><MaterialSummary source={r} variant="line" /></button>
            {/each}
          {/each}
        {/if}
      {/each}
    {:else}
      <p class="muted">{search ? 'No material matches.' : 'No recipes yet.'}</p>
    {/each}
  </div>
  <div class="library-foot">
    <div class="import-buttons">
      <label class="btn btn-ghost">Import files<input type="file" multiple accept=".xml,.png,.jpg,.jpeg" hidden onchange={chosen}></label>
      <label class="btn btn-ghost">Import folder<input type="file" multiple webkitdirectory hidden onchange={chosen}></label>
    </div>
    <span class="muted">{dragging ? 'Drop recipe files or folders to review them' : `${plural(materials.length, 'material')} · ${plural(recipes.length, 'recipe')} · or drop files here`}</span>
  </div>
</aside>

{#if importing}
  <ImportRecipes files={importing} onclose={() => (importing = null)} />
{/if}
