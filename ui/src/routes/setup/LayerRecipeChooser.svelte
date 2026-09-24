<script lang="ts">
  // What runs one layer: the job's recipe, another recipe for the same
  // laser, or nothing at all.
  import Modal from '../../components/Modal.svelte';
  import { api } from '../../api/client';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { explain, recipeLabel } from '../../lib/format';
  import type { DraftLayer, RecipeView } from '../../api';

  let { layer, onclose }: { layer: DraftLayer; onclose: () => void } = $props();
  const draft = $derived(server.doc!.draft!);
  // Recipes for the job's laser, the job's own material first.
  const recipes = $derived.by(() => {
    const job = draft.recipe;
    const all = (server.doc?.library.recipes ?? []).filter((r) => !job || r.laser === job.laser);
    const same = (r: RecipeView) => (job && r.name === job.name ? 0 : 1);
    return [...all].sort((a, b) => same(a) - same(b) || a.name.localeCompare(b.name) || a.thickness_mm - b.thickness_mm);
  });
  async function choose(recipe: string | null | 'off'): Promise<void> {
    // Closing clears the parent's layer, so take what this needs first.
    const { name, output } = layer;
    onclose();
    try {
      if (recipe === 'off') { await api.changeLayers({ kind: 'output', layer: name, on: false }); return; }
      if (!output) await api.changeLayers({ kind: 'output', layer: name, on: true });
      await api.changeLayers({ kind: 'recipe', layer: name, recipe });
    } catch (error) { ui.say(explain(error), true); }
  }
</script>

<Modal title="Layer {layer.name}" {onclose}>
  <div class="choices">
    <button class="choice" class:on={layer.output && layer.chosen && !layer.recipe} onclick={() => choose(null)}>
      <strong>Job recipe</strong><span>{draft.recipe ? recipeLabel(draft.recipe) : 'The material chosen for the job'}</span>
    </button>
    <button class="choice" class:on={!layer.output} onclick={() => choose('off')}>
      <strong>Off</strong><span>Leave this layer uncut</span>
    </button>
    {#each recipes as recipe (recipe.id)}
      <button class="choice" class:on={layer.recipe?.id === recipe.id} onclick={() => choose(recipe.id)}>
        <strong>{recipeLabel(recipe)}</strong>
      </button>
    {/each}
  </div>
</Modal>

<style>
  .choices { display:grid; gap:8px; max-height:60vh; overflow:auto; }
  .choice { min-height:56px; display:grid; gap:3px; padding:10px 14px; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); text-align:left; cursor:pointer; }
  .choice.on { border-color:var(--accent); background:var(--accent-soft); }
  .choice span { font-size:var(--t-sm); color:var(--ink-3); }
</style>
