<script lang="ts">
  // The job's layers: shown or hidden here, and how each is cut. With more
  // than one layer to cut, each needs a recipe or Ignore before the job
  // compiles. Names are the operator's to change; shapes move between
  // layers by picking them on the drawing.
  import Modal from '../../components/Modal.svelte';
  import { api } from '../../api/client';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { explain, plural, recipeLabel } from '../../lib/format';
  import type { DraftLayer, RecipeView } from '../../api';

  let { layers, onpick, onclose }: {
    layers: DraftLayer[];
    /** Starts picking shapes to move to a layer. */
    onpick: () => void;
    onclose: () => void;
  } = $props();

  const draft = $derived(server.doc?.draft);
  /** Several layers to cut: each must be chosen. */
  const asking = $derived(layers.filter((l) => !l.ignored).length > 1);
  let choosing = $state<DraftLayer | null>(null);
  let busy = $state(false);
  // Recipes for the job's laser, the job's own material first.
  const recipes = $derived.by(() => {
    const job = draft?.recipe;
    const all = (server.doc?.library.recipes ?? []).filter((r) => !job || r.laser === job.laser);
    const same = (r: RecipeView) => (job && r.name === job.name ? 0 : 1);
    return [...all].sort((a, b) => same(a) - same(b) || a.name.localeCompare(b.name) || a.thickness_mm - b.thickness_mm);
  });

  async function change(run: () => Promise<unknown>): Promise<void> {
    busy = true;
    try { await run(); } catch (error) { ui.say(explain(error), true); } finally { busy = false; }
  }
  const cut = (layer: DraftLayer, recipe: string | null, engrave: boolean) =>
    change(() => api.changeLayers({ kind: 'cut', layer: layer.name, recipe, engrave }));
  function choose(recipe: string | null | 'ignore'): void {
    const layer = choosing;
    choosing = null;
    if (!layer) return;
    if (recipe === 'ignore') void change(() => api.changeLayers({ kind: 'ignore', layer: layer.name }));
    else void cut(layer, recipe, layer.engrave);
  }
  function rename(layer: DraftLayer): void {
    osk.text('Layer name', layer.name, (to) => {
      if (to.trim() && to.trim() !== layer.name) void change(() => api.changeLayers({ kind: 'rename', from: layer.name, to }));
    });
  }
  /** What cuts the layer, in a word or two. */
  function how(layer: DraftLayer): string {
    if (layer.ignored) return 'Ignored';
    if (!layer.chosen) return asking ? 'Choose a recipe' : 'Job recipe';
    return layer.recipe ? recipeLabel(layer.recipe) : 'Job recipe';
  }

  const hidden = (layer: string) => ui.hiddenDrawingLayers.includes(layer);
  function toggleHidden(layer: string): void {
    const off = !hidden(layer);
    ui.hiddenDrawingLayers = off
      ? [...new Set([...ui.hiddenDrawingLayers, layer])]
      : ui.hiddenDrawingLayers.filter((l) => l !== layer);
  }
</script>

<div class="layers-pop">
  <div class="lp-head"><h3>Layers</h3><button class="link" onclick={onclose}>Close</button></div>
  {#if asking && layers.some((l) => !l.chosen)}<p class="lp-note">Choose a recipe or Ignore for each layer.</p>{/if}
  {#each layers as layer (layer.name)}
    <div class="lrow" class:hidden-layer={hidden(layer.name)}>
      <button class="eye" class:off={hidden(layer.name)} title="Show or hide" aria-label="Show or hide {layer.name}"
        onclick={() => toggleHidden(layer.name)}>
        <span class="sw" style="background:{layer.ignored ? 'var(--ink-3)' : 'var(--ink)'}"></span>
      </button>
      <button class="lname-btn" disabled={busy} onclick={() => rename(layer)} aria-label="Rename {layer.name}">
        <span class="lname">{layer.name}</span>
        <span class="lmeta">{plural(layer.contours, 'shape')}</span>
      </button>
      <button class="how" class:warn={asking && !layer.chosen} disabled={busy} onclick={() => (choosing = layer)}>{how(layer)}</button>
      {#if layer.chosen && !layer.ignored}
        <div class="engrave">
          <span>Engrave<small>On the surface, before the cuts</small></span>
          <button class="switch" class:on={layer.engrave} disabled={busy} aria-label="Engrave {layer.name}"
            onclick={() => cut(layer, layer.recipe?.id ?? null, !layer.engrave)}></button>
        </div>
      {/if}
    </div>
  {/each}
  <button class="btn btn-ghost block" disabled={busy} onclick={onpick}><i class="ic ic-layers"></i>Move shapes to a layer…</button>
</div>

{#if choosing}
  <Modal title="Layer {choosing.name}" onclose={() => (choosing = null)}>
    <div class="recipe-choices">
      <button class="choice" class:on={choosing.chosen && !choosing.ignored && !choosing.recipe} onclick={() => choose(null)}>
        <strong>Job recipe</strong><span>{draft?.recipe ? recipeLabel(draft.recipe) : 'The material chosen for the job'}</span>
      </button>
      <button class="choice" class:on={choosing.ignored} onclick={() => choose('ignore')}>
        <strong>Ignore</strong><span>Leave this layer uncut</span>
      </button>
      {#each recipes as recipe (recipe.id)}
        <button class="choice" class:on={choosing.recipe?.id === recipe.id} onclick={() => choose(recipe.id)}>
          <strong>{recipeLabel(recipe)}</strong>
        </button>
      {/each}
    </div>
  </Modal>
{/if}

<style>
  .lp-note { margin:0; padding:0 4px; color:var(--warn); font-size:var(--t-sm); }
  .lname-btn { min-height:44px; display:grid; gap:2px; padding:0 4px; border:0; background:transparent; color:var(--ink); text-align:left; cursor:pointer; min-width:0; }
  .lname-btn .lname { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .how { min-height:44px; max-width:180px; padding:0 12px; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); font:inherit; font-size:var(--t-sm); font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; cursor:pointer; }
  .how.warn { border-color:var(--warn); color:var(--warn); }
  .engrave { grid-column:1 / -1; display:flex; align-items:center; justify-content:space-between; gap:10px; padding:0 4px; font-size:var(--t-sm); font-weight:600; }
  .engrave small { display:block; font-weight:400; color:var(--ink-3); }
  .recipe-choices { display:grid; gap:8px; max-height:60vh; overflow:auto; }
  .choice { min-height:56px; display:grid; gap:3px; padding:10px 14px; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); text-align:left; cursor:pointer; }
  .choice.on { border-color:var(--accent); background:var(--accent-soft); }
  .choice span { font-size:var(--t-sm); color:var(--ink-3); }
</style>
