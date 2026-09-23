<script lang="ts">
  import { distance, quantity, unitLabel } from '../../lib/units.svelte';
  import Modal from '../../components/Modal.svelte';
  import MaterialSummary from '../../components/MaterialSummary.svelte';
  import { api } from '../../api/client';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { explain, laserLabel, recipeLabel } from '../../lib/format';
  import { pictureOf, materialsOf, materialKey } from '../../lib/materials';
  import type { RecipeView } from '../../api';

  let { onclose }: { onclose: () => void } = $props();
  const doc = $derived(server.doc!);
  const recipes = $derived(doc.library.recipes);
  const recent = $derived.by(() => {
    const seen = new Set<string>();
    return [...doc.library.jobs].sort((a, b) => b.updated - a.updated).map((j) => j.recipe).filter((r) => { const live = recipes.find((x) => x.key === r.key); if (!live || seen.has(live.id)) return false; seen.add(live.id); return true; }).map((r) => recipes.find((x) => x.key === r.key)!).slice(0, 5);
  });
  const materials = $derived(materialsOf(recipes));

  let pick = $state<RecipeView | null>(null);
  $effect(() => { if (!pick) pick = recipes.find((r) => r.id === doc.draft?.recipe?.id) ?? recipes[0] ?? null; });
  const group = $derived(pick ? recipes.filter((r) => materialKey(r) === materialKey(pick!)) : []);
  const thicknesses = $derived([...new Set(group.map((r) => r.thickness_mm))].sort((a, b) => a - b));
  const gases = $derived(group.filter((r) => r.thickness_mm === pick?.thickness_mm));

  function choose(r: RecipeView): void { pick = r; }
  function use(): void {
    if (!pick) return;
    api.setRecipe(pick.id).then(() => { ui.say(`Job material: ${recipeLabel(pick)}`); onclose(); }).catch((error) => ui.say(explain(error), true));
  }
</script>

<Modal title="Choose material for this job" wide {onclose}>
  <div class="sheet">
    <div class="sheet-list">
      {#if recent.length}<h3>Recent</h3><div class="recent">{#each recent as r}<button class="chip" class:on={pick?.id === r.id} onclick={() => choose(r)}>{recipeLabel(r)}</button>{/each}</div>{/if}
      <h3 style="margin-top:6px">All materials</h3>
      {#each materials as m}
        {@const picture = pictureOf(m.name, m.recipes.find((r) => r.photo)?.photo)}
        <div class="mat-row" class:on={pick && materialKey(pick) === m.key} role="button" tabindex="0" onclick={() => choose(m.recipes[0]!)} onkeydown={(e) => { if (e.key === 'Enter') choose(m.recipes[0]!); }}>
          {#if picture}<img class="swatch" src={picture} alt="">{:else}<div class="swatch" style="background:var(--panel-2)"></div>{/if}
          <div><div class="name">{m.recipes.some((r) => r.favourite) ? '★ ' : ''}{m.name}</div><div class="meta">{laserLabel(m.recipes[0]!.laser)} · {[...new Set(m.recipes.map((r) => r.thickness_mm))].sort((a, b) => a - b).map(t => distance(t)).join(', ')} {unitLabel('mm')} · {[...new Set(m.recipes.map((r) => r.gas))].join(' / ')}</div></div>
          <span class="tag">{m.recipes.length}</span>
        </div>
      {:else}
        <p class="muted">No recipes yet. Add one on the Materials page.</p>
      {/each}
    </div>
    <div class="sheet-side">
      {#if pick}
        {@const picture = pictureOf(pick.name, pick.photo)}
        <div class="side-title">{#if picture}<img class="swatch" src={picture} alt="">{:else}<div class="swatch" style="background:var(--panel-2)"></div>{/if}<div><h2>{pick.name}</h2><div class="muted">{laserLabel(pick.laser)}</div></div></div>
        <div class="field"><h3>Thickness</h3><div class="chips">{#each thicknesses as t}<button class="chip" class:on={t === pick.thickness_mm} onclick={() => choose(group.find((r) => r.thickness_mm === t)!)}>{quantity(t, 'mm')}</button>{/each}</div></div>
        <div class="field"><h3>Assist gas</h3><div class="chips">{#each gases as r}<button class="chip" class:on={r.id === pick.id} onclick={() => choose(r)}>{r.gas}</button>{/each}</div></div>
        <MaterialSummary source={pick} />
        <button class="btn btn-primary lg block" onclick={use}>Use {pick.name} {quantity(pick.thickness_mm, 'mm')}</button>
        <button class="link" onclick={() => { onclose(); ui.selectedRecipe = pick!.id; ui.tab = 'materials'; }}>Edit recipes in the library</button>
      {/if}
    </div>
  </div>
</Modal>
