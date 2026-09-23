<script lang="ts">
  // One machining feature's settings: every field the server takes, in
  // the words the operator uses. Places on the drawing are picked on the
  // canvas; the panel only starts and ends the picking. Each feature's form
  // is its own component under features/.
  import { featureEdits } from '../../stores/feature-edits';
  import { server } from '../../stores/server.svelte';
  import { ui, type FeatureId } from '../../stores/ui.svelte';
  import { ago, explain, recipeLabel } from '../../lib/format';
  import { TOOLS, defaultBridges, defaultCommon, defaultCooling, defaultJoints, defaultKerf, defaultLeads, isOn } from '../../lib/features';
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
  const features = $derived(featureEdits.value(draft));
  const on = $derived(isOn(features, id));
  let tab = $state(0);
  $effect(() => { void id; tab = 0; });

  const set: SetFeatures = (change) => {
    featureEdits.change(change).catch((error) => ui.say(explain(error), true));
  };

  /** Starts placing on the drawing; the canvas takes it from here. */
  function pick(feature: PickFeature): void {
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
    <h2>{compact ? 'Enabled' : tool.name}</h2>
    {#if tool.optional}<button class="switch" class:on={on} onclick={ontoggle} aria-label="On"></button>{/if}
    {#if !compact}<button class="btn btn-ghost" onclick={close}>Close</button>{/if}
  </div>
{/if}
<p class="feat-desc">{tool.desc}</p>
{#if id === 'order'}<CutOrderPreview preview={draft.preview} disabled={!!ui.picking} bind:progress={orderProgress} />{/if}
{#if featureEdits.error}
  <p class="muted">{featureEdits.error} <button class="link" onclick={() => featureEdits.discard()}>Discard refused edits</button></p>
{/if}
{#if id === 'leads'}<Tabs names={['Entry', 'Exit']} bind:active={tab} />{/if}
{#if id === 'leads'}<p class="muted" style="font-size:var(--t-sm)">Drag a lead to edit it. Selected copies update the matching lead.</p>{/if}
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
