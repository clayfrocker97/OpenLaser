<script lang="ts">
  import { unitLabel } from '../../lib/units.svelte';
  // The imported values: every attribute the file holds, read only, under
  // what the process does with it now. Nothing here is a control.
  import { bare, field, stageOf, status, type Editor, type Status } from '../../lib/recipe';
  import type { RecipeView } from '../../api';

  let { ed, recipe }: { ed: Editor; recipe: RecipeView } = $props();
  const ORDER: Array<[Status, string, string]> = [
    ['unresolved', 'Unresolved', 'A selection the machine has no valve for; choose a wired gas.'],
    ['unavailable', 'Not run on this machine', 'Values retained for manual optical focus, graph appearance or unused options. These do not change the cutting process.'],
    ['inactive', 'Inactive', 'Stored for a mode, stage or option that is off.'],
    ['active', 'Active', 'Read by the process as the recipe is set now.'],
  ];
  const context = (key: string) => { const s = stageOf(key); return s === null ? '' : `bank ${s} · `; };
  const groups = $derived(ORDER.map(([kind, title, help]) => ({ kind, title, help, keys: Object.keys(recipe.attributes).filter((key) => key !== 'Note' && status(key, ed.values, ed.a) === kind).sort() })).filter((g) => g.keys.length));
</script>

<div class="inspect">
  {#each groups as g (g.kind)}
    <h3>{g.title} · {g.keys.length}</h3>
    <p class="muted">{g.help}</p>
    {#each g.keys as key (key)}
      <div class="row"><span><span class="mono">{key}</span> · {context(key)}{field(key).label}</span><span><b>{bare(key, recipe.attributes[key] ?? '')}</b>{#if field(key).unit && field(key).kind === 'number'}{' '}{unitLabel(field(key).unit ?? '')}{/if}<span class="state-tag {g.kind}">{g.title}</span></span></div>
    {/each}
  {/each}
</div>
