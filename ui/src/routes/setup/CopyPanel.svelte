<script lang="ts">
  import { api } from '../../api/client';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { explain, recipeLabel } from '../../lib/format';

  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft!);
  const key = $derived(draft.recipe?.key ?? '');
  const jobs = $derived([...doc.library.jobs].sort((a, b) => Number(b.recipe.key === key) - Number(a.recipe.key === key) || b.updated - a.updated));
  const same = $derived(jobs.filter((j) => j.recipe.key === key));
  const others = $derived(jobs.filter((j) => j.recipe.key !== key));

  function copy(id: string, name: string): void {
    api.copyFeatures(id).then(() => { ui.setupPanel = null; ui.say(`Machining settings copied from ${name}.`); }).catch((error) => ui.say(explain(error), true));
  }
</script>

<div class="feat-head"><h2>Copy machining from a job</h2><button class="btn btn-ghost" onclick={() => (ui.setupPanel = null)}>Close</button></div>
<p class="feat-desc">Copy machining defaults. Place manual points and bridges again.</p>
<div class="stack">
  {#if same.length}<div class="divider same">Same recipe · {recipeLabel(draft.recipe)}</div>{/if}
  {#each same as j}
    <div class="mat-row same" role="button" tabindex="0" onclick={() => copy(j.id, j.name)} onkeydown={(e) => { if (e.key === 'Enter') copy(j.id, j.name); }}><div class="swatch" style="background:var(--panel-2)"></div><div class="minw0"><div class="name"><span class="txt">{j.name}</span></div><div class="meta">{recipeLabel(j.recipe)} · {j.features_on.join(', ') || 'nothing on'}</div></div><span class="chev"><i class="ic ic-chev-right"></i></span></div>
  {/each}
  {#if others.length}<div class="divider">Other recipes</div>{/if}
  {#each others as j}
    <div class="mat-row" role="button" tabindex="0" onclick={() => copy(j.id, j.name)} onkeydown={(e) => { if (e.key === 'Enter') copy(j.id, j.name); }}><div class="swatch" style="background:var(--panel-2)"></div><div class="minw0"><div class="name"><span class="txt">{j.name}</span></div><div class="meta">{recipeLabel(j.recipe)} · {j.features_on.join(', ') || 'nothing on'}</div></div><span class="chev"><i class="ic ic-chev-right"></i></span></div>
  {/each}
  {#if !jobs.length}<p class="muted">No saved jobs yet.</p>{/if}
</div>
