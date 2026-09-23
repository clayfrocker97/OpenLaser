<script lang="ts">
  import { quantity } from '../lib/units.svelte';
  // The material library, full screen: the recipes as a tree on the
  // left and the selected recipe's values beside it. A recipe is one of
  // the vendor's layer banks: imported from the process library folder,
  // copied to another thickness, or made from a bank of the machine
  // files. Edits stay staged until Save; opening another recipe over
  // them asks whether to save, discard or stay.
  import Library from './materials/Library.svelte';
  import Recipe from './materials/Recipe.svelte';
  import NewRecipe from './materials/NewRecipe.svelte';
  import Modal from '../components/Modal.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain } from '../lib/format';
  import { recipeEdits } from '../lib/recipe-edits.svelte';
  import { materialKey } from '../lib/materials';
  import type { RecipeView } from '../api';

  let { compact = false }: { compact?: boolean } = $props();
  let editing = $state(false);

  const doc = $derived(server.doc!);
  const recipes = $derived(doc.library.recipes);

  const recipe = $derived(recipes.find((r) => r.id === ui.selectedRecipe) ?? recipes[0] ?? null);
  const edits = $derived(recipe ? recipeEdits.attributes(recipe.id) : {});
  const filmEdit = $derived(recipe ? recipeEdits.film(recipe.id) : undefined);
  let adding = $state(false);
  /** The recipe the operator asked to open over staged changes. */
  let leaving = $state<RecipeView | null>(null);
  const values = $derived({ ...(recipe?.attributes ?? {}), ...edits });
  const pending = $derived(Object.keys(edits).length + (filmEdit === undefined ? 0 : 1));
  const filmId = $derived(filmEdit === undefined ? (recipe?.film ?? null) : filmEdit);
  const film = $derived(recipes.find((r) => r.id === filmId) ?? null);

  function select(r: RecipeView): boolean {
    if (r.id !== recipe?.id && pending) { leaving = r; return false; }
    ui.selectedRecipe = r.id;
    adding = false;
    editing = true;
    return true;
  }
  function set(key: string, value: string): void {
    if (recipe) recipeEdits.set(recipe, key, value);
  }
  function setFilm(id: string | null): void {
    if (recipe) recipeEdits.setFilm(recipe, id);
  }
  async function save(): Promise<boolean> {
    if (!recipe) return false;
    const submitted = recipeEdits.begin(recipe.id);
    if (!submitted) return false;
    try {
      await api.updateRecipe(submitted.id, submitted.change);
      recipeEdits.finish(submitted, true);
      const remaining = recipeEdits.count(submitted.id);
      ui.say(remaining ? 'Submitted changes saved. Your newer edits are still staged.' : 'Recipe saved.');
      return remaining === 0;
    } catch (error) {
      recipeEdits.finish(submitted, false);
      ui.say(explain(error), true);
      return false;
    }
  }
  async function discard(): Promise<void> {
    if (!recipe) return;
    const count = recipeEdits.count(recipe.id);
    const confirmed = await ui.confirm({ title: 'Discard these recipe edits?', body: `${count} edited ${count === 1 ? 'value is' : 'values are'} thrown away; ${recipe.name} keeps its saved values.`, confirm: 'Discard edits', danger: true });
    if (confirmed) recipeEdits.discard(recipe.id);
  }
  /** Another thickness of the recipe: a copy of its values. */
  function copy(): void {
    if (!recipe) return;
    if (pending) { ui.say('Save or discard the changes first.', true); return; }
    const source = recipe;
    osk.number('New thickness', source.thickness_mm, 'mm', async (v) => {
      if (v <= 0) return;
      if (recipes.some((r) => materialKey(r) === materialKey(source) && r.gas === source.gas && r.thickness_mm === v)) { ui.say(`${source.name} ${quantity(v, 'mm')} ${source.gas} is already there.`, true); return; }
      try {
        ui.selectedRecipe = (await api.duplicateRecipe(source.id, v)).id;
        ui.say(`${source.name} ${quantity(v, 'mm')} added; tune its values.`);
      } catch (error) {
        ui.say(explain(error), true);
      }
    });
  }
</script>

{#if !compact || !editing}<Library {recipes} selected={compact ? null : recipe} selectOnExpand={!compact} onselect={select} onadd={() => { if (pending) ui.say('Save or discard the changes first.', true); else { adding = true; editing = true; } }} />{/if}
{#if compact && editing}<button class="btn btn-ghost" onclick={() => { editing = false; adding = false; }}>‹ Materials</button>{/if}
{#if !compact || editing}
{#if adding}
  <NewRecipe onclose={() => (adding = false)} onadded={(id) => { adding = false; ui.selectedRecipe = id; }} />
{:else if recipe}
  <Recipe {recipe} saving={!!recipeEdits.saving[recipe.id]} {values} {edits} {film} onset={set} onfilm={setFilm} oncopy={copy} onsave={save} ondiscard={discard} />
{:else}
  <section class="panel main"><div class="empty"><h2>No recipes yet</h2>Import a process library or create a recipe from a machine bank.</div></section>
{/if}
{/if}

{#if leaving}
  {@const next = leaving}
  <Modal title="Save this recipe?" onclose={() => (leaving = null)}>
    <p class="muted">This recipe has {pending} staged change{pending === 1 ? '' : 's'}. Save or discard them before opening {next.name} · {quantity(next.thickness_mm, 'mm')}.</p>
    <div class="row">
      <button class="btn btn-ghost" onclick={() => (leaving = null)}>Stay</button>
      <button class="btn btn-ghost" onclick={() => { discard(); leaving = null; select(next); }}>Discard</button>
      <button class="btn btn-primary" onclick={async () => { if (await save()) { leaving = null; select(next); } }}>Save and open</button>
    </div>
  </Modal>
{/if}
