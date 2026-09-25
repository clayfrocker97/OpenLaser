<script lang="ts">
  // One machining feature's settings: every field the server takes, in
  // the words the operator uses. Places on the drawing are picked on the
  // canvas; the panel only starts and ends the picking. Each feature's form
  // is its own component under features/.
  import { featureEdits } from '../../stores/feature-edits';
  import { server } from '../../stores/server.svelte';
  import { ui, type FeatureId } from '../../stores/ui.svelte';
  import { ago, explain, recipeLabel } from '../../lib/format';
  import { TOOLS, defaultBridges, defaultCommon, defaultCooling, defaultJoints, defaultKerf, defaultLeads, isOn, toggled } from '../../lib/features';
  import Tabs from '../../components/params/Tabs.svelte';
  import CutOrderPreview from './CutOrderPreview.svelte';
  import BridgesForm from './features/BridgesForm.svelte';
  import CommonForm from './features/CommonForm.svelte';
  import CoolingForm from './features/CoolingForm.svelte';
  import JointsForm from './features/JointsForm.svelte';
  import KerfForm from './features/KerfForm.svelte';
  import LeadsForm from './features/LeadsForm.svelte';
  import OrderForm from './features/OrderForm.svelte';
  import StartForm from './features/StartForm.svelte';
  import type { PickFeature, SetFeatures } from './features/form';
  import type { Features } from '../../api';
  import { hasOwn, LAYER_MACHINING, scoped, setOwn, swatchColor } from '../../lib/drawing-layers';

  let { id, ontoggle, selectedContours, orderProgress = $bindable(0), compact = false }: {
    id: FeatureId;
    ontoggle: () => void;
    selectedContours: number[];
    orderProgress?: number;
    compact?: boolean;
  } = $props();
  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft!);
  const tool = $derived(TOOLS.find((t) => t.id === id)!);
  const base = $derived(featureEdits.value(draft));
  /** Leads, joints, cooling, kerf and start can be a layer's own. */
  const perLayer = $derived((LAYER_MACHINING as readonly string[]).includes(id) && draft.layers.length > 1);
  const scope = $derived(perLayer && draft.layers.some((l) => l.name === ui.machiningLayer) ? ui.machiningLayer : null);
  const features = $derived(scope ? scoped(base, scope) : base);
  const own = $derived(scope ? hasOwn(base, scope) : false);
  const on = $derived(isOn(features, id));
  const order = $derived(draft.layers.map((l) => l.name));
  let tab = $state(0);
  $effect(() => { void id; tab = 0; });

  /** An edit to the job's machining, or, with a layer chosen, to that
   *  layer's own, which the edit gives it when it has none yet. */
  const set: SetFeatures = (change) => {
    const layer = scope;
    const edit = layer
      ? (f: Features) => { const view = structuredClone(scoped(f, layer)); change(view); setOwn(f, layer, view, order); }
      : change;
    featureEdits.change(edit).catch((error) => ui.say(explain(error), true));
  };
  /** The layer goes back to the job's machining. */
  const useJob = () => { const layer = scope; if (layer) featureEdits.change((f) => setOwn(f, layer, null, order)).catch((error) => ui.say(explain(error), true)); };
  /** On or off; for a layer, its own. */
  function toggle(): void {
    if (!scope) { ontoggle(); return; }
    set((f) => { Object.assign(f, toggled(f, id)); });
  }

  /** Starts placing on the drawing; the canvas takes it from here. Places
   *  picked on the drawing are the job's, whichever layer they are on. */
  function pick(feature: PickFeature): void {
    if (scope) { ui.say('Places picked on the drawing apply to every layer. Choose All layers to pick them.'); return; }
    ui.picking = { feature, first: null, order: [], revision: draft.revision, features: structuredClone($state.snapshot(features)), marks: [] };
  }

  function reset(): void {
    set((f) => {
      if (id === 'leads') f.leads = defaultLeads();
      if (id === 'joints') f.joints = defaultJoints();
      if (id === 'cooling') f.cooling = defaultCooling();
      if (id === 'kerf') f.kerf = defaultKerf();
      if (id === 'bridges') f.bridges = defaultBridges();
      if (id === 'common' && f.common) f.common = defaultCommon(f.common.contours);
      if (id === 'order') f.order = { strategy: 'as_drawn', inner_first: true, circles_first: false, spread_heat: false };
      if (id === 'start') { f.start = { position: 'automatic', direction: 'keep', spots: [] }; f.seam = 'seal'; }
    });
  }
  const close = () => (ui.setupPanel = null);
</script>

{#if !compact || tool.optional}
  <div class="feat-head">
    <h2 class="feat-title"><i class="ic ic-tool-{id}"></i>{compact ? 'Enabled' : tool.name}</h2>
    {#if tool.optional}<button class="switch" class:on={on} onclick={toggle} aria-label="On"></button>{/if}
    {#if !compact}<button class="btn btn-ghost" onclick={close}>Close</button>{/if}
  </div>
{/if}
<p class="feat-desc">{tool.desc}</p>
{#if perLayer}
  <div class="scope" role="group" aria-label="Applies to">
    <button class="chip" class:on={!scope} onclick={() => (ui.machiningLayer = null)}>All layers</button>
    {#each draft.layers as layer (layer.name)}
      <button class="chip" class:on={scope === layer.name} onclick={() => (ui.machiningLayer = layer.name)}>
        <span class="layer-swatch" style:--layer={swatchColor(draft.layers, layer.name)}></span>{layer.name}{hasOwn(base, layer.name) ? ' ·' : ''}
      </button>
    {/each}
  </div>
  {#if scope}
    <p class="muted scope-note">
      {#if own}{scope} has its own settings. <button class="link" onclick={useJob}>Use the job's</button>
      {:else if draft.layers.find((l) => l.name === scope)?.mode === 'mark'}{scope} is a mark, so it has none; a change gives it its own.
      {:else}{scope} uses the job's settings; a change gives it its own.{/if}
    </p>
  {:else if draft.layers.some((l) => hasOwn(base, l.name))}
    <p class="muted scope-note">Layers marked · keep their own settings.</p>
  {/if}
{/if}
{#if id === 'order'}<CutOrderPreview preview={draft.preview} disabled={!!ui.picking} bind:progress={orderProgress} />{/if}
{#if featureEdits.error}
  <p class="muted">{featureEdits.error} <button class="link" onclick={() => featureEdits.discard()}>Discard refused edits</button></p>
{/if}
{#if id === 'leads'}<Tabs names={['Entry', 'Exit']} bind:active={tab} />{/if}
{#if id === 'leads'}<p class="muted lead-hint">Drag a lead to edit it. Selected copies update the matching lead.</p>{/if}
{#if id === 'joints' || id === 'start'}<Tabs names={['Basic', 'More']} bind:active={tab} />{/if}
<div class="stack" style="opacity:{on ? 1 : 0.45}">
  {#if id === 'leads'}
    <LeadsForm {features} {on} {tab} {selectedContours} {set} />
  {:else if id === 'joints'}
    <JointsForm {features} {on} {tab} contours={draft.placed.length} {set} {pick} />
  {:else if id === 'cooling'}
    <CoolingForm {features} {on} contours={draft.placed.length} {set} {pick} />
  {:else if id === 'kerf'}
    <KerfForm {features} {on} {set} />
  {:else if id === 'bridges'}
    <BridgesForm {features} {on} {set} {pick} />
  {:else if id === 'common'}
    <CommonForm {features} {on} {selectedContours} error={draft.error} {set} />
  {:else if id === 'order'}
    <OrderForm {features} {on} preview={draft.preview} {set} {pick} />
  {:else if id === 'start'}
    <StartForm {features} {on} {tab} {set} {pick} />
  {/if}
</div>
<div class="side-foot">
  <div class="source">
    {#if draft.feature_source}Prefilled from <strong>{draft.feature_source.name}</strong> ({ago(draft.feature_source.at)})
      on {recipeLabel(draft.recipe)}. Saved with this job; the next job on this recipe starts from these.{:else}First job on
      {recipeLabel(draft.recipe)}. Saved with this job; the next job on this recipe starts from these.{/if}
  </div>
  <div class="row">
    <button class="btn btn-ghost" onclick={reset}>Reset to defaults</button>
    <button class="btn btn-primary" onclick={close}>Done</button>
  </div>
</div>

<style>
  .lead-hint { font-size:var(--t-sm); }
  .feat-title { display:flex; align-items:center; gap:10px; }
  .feat-title .ic { width:24px; height:24px; color:var(--accent-2); }
  .scope { display:flex; flex-wrap:wrap; gap:6px; margin:0 0 8px; }
  .scope .chip { min-height:44px; display:inline-flex; align-items:center; gap:6px; }
  .layer-swatch { width:12px; height:12px; border-radius:3px; background:var(--layer); }
  .scope-note { font-size:var(--t-sm); margin:0 0 10px; }
</style>
