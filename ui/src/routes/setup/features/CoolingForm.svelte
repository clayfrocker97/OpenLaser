<script lang="ts">
  // Cooling stops: pauses in the cut that let the heat spread.
  import Chips from '../../../components/params/Chips.svelte';
  import Placed from '../../../components/params/Placed.svelte';
  import Stepper from '../../../components/params/Stepper.svelte';
  import Toggle from '../../../components/params/Toggle.svelte';
  import { defaultCooling } from '../../../lib/features';
  import type { Features } from '../../../api';
  import { askPercentages, spots, type PickFeature, type SetFeatures } from './form';

  let { features, on, contours, set, pick }: {
    features: Features;
    on: boolean;
    /** How many contours the drawing has. */
    contours: number;
    set: SetFeatures;
    pick: (feature: PickFeature) => void;
  } = $props();

  const PLACEMENTS = [['automatic', 'Automatic'], ['manual', 'Picked']] as const;
  const cooling = $derived(features.cooling ?? defaultCooling());

  const setDwell = (v: number) => set((f) => { f.cooling!.dwell = Math.max(1, Math.round(v * 1000)); });
  function setPlacement(v: string): void {
    set((f) => {
      f.cooling!.placement = v === 'automatic'
        ? { automatic: { at_start: true, corners_below: 60 } }
        : { manual: spots(f.cooling!.placement) };
    });
  }
  /** Changes the automatic placement, if that is what the cooling uses. */
  function editAutomatic(change: (auto: { at_start: boolean; corners_below: number | null }) => void): void {
    set((f) => { const p = f.cooling!.placement; if ('automatic' in p) change(p.automatic); });
  }
  const setAtStart = (v: boolean) => editAutomatic((a) => { a.at_start = v; });
  const setCorners = (v: boolean) => editAutomatic((a) => { a.corners_below = v ? 60 : null; });
  const setCornerAngle = (v: number) => editAutomatic((a) => { a.corners_below = Math.min(180, v); });
  const clearPicked = () => set((f) => { f.cooling!.placement = { manual: [] }; });
</script>

<button class="btn btn-soft" onclick={() => askPercentages(features, 'cooling', contours, set)} disabled={!on}>Enter positions (%)…</button>
<Stepper label="Dwell" value={cooling.dwell / 1000} unit="s" step={0.1} disabled={!on} apply={setDwell} />
<Chips label="Placement" options={PLACEMENTS} current={'automatic' in cooling.placement ? 'automatic' : 'manual'} disabled={!on} apply={setPlacement} />
{#if 'automatic' in cooling.placement}
  {@const auto = cooling.placement.automatic}
  <Toggle label="Cool after the pierce" value={auto.at_start} disabled={!on} apply={setAtStart} />
  <Toggle label="Cool at sharp corners" value={auto.corners_below !== null} disabled={!on} apply={setCorners} />
  {#if auto.corners_below !== null}
    <Stepper label="Corners sharper than" value={auto.corners_below} unit="°" step={5} disabled={!on} apply={setCornerAngle} />
  {/if}
{:else}
  <Placed count={cooling.placement.manual.length} what="stop" disabled={!on} onclear={clearPicked} onpick={() => pick('cooling')} />
{/if}
