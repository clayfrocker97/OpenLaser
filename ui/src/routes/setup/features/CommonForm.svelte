<script lang="ts">
  // Common edges: adjacent contours that share one cut.
  import Stepper from '../../../components/params/Stepper.svelte';
  import Toggle from '../../../components/params/Toggle.svelte';
  import { defaultCommon } from '../../../lib/features';
  import { plural } from '../../../lib/format';
  import { ui } from '../../../stores/ui.svelte';
  import type { Features } from '../../../api';
  import type { SetFeatures } from './form';

  let { features, on, selectedContours, error, set }: {
    features: Features;
    on: boolean;
    selectedContours: number[];
    /** Why the drawing could not be prepared, if it could not. */
    error: string | null;
    set: SetFeatures;
  } = $props();

  const common = $derived(features.common ?? defaultCommon(selectedContours));
  const summary = $derived(features.common
    ? `${plural(common.contours.length, 'contour')} in this common-edge group`
    : 'Select adjacent contours on the drawing, then switch on common edges.');
  const useSelected = () => set(f => { if (f.common) f.common.contours = [...selectedContours]; });
  const setTolerance = (v: number) => set(f => { f.common!.tolerance = v; });
  const setOvercut = (v: boolean) => set(f => { f.common!.allow_overcut = v; });
</script>

<div class="param"><div class="lbl">{summary}</div></div>
<button class="btn btn-soft" disabled={!on || selectedContours.length < 2 || !!ui.picking} onclick={useSelected}>Use selected contours ({selectedContours.length})</button>
<Stepper label="Matching tolerance" value={common.tolerance} unit="mm" step={0.01} disabled={!on} apply={setTolerance} />
<Toggle label="Allow overcut between remaining spans" value={common.allow_overcut} disabled={!on} apply={setOvercut} />
{#if error}<p class="warn-text">{error}</p>{/if}
