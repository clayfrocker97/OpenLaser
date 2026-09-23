<script lang="ts">
  // Joints (micro-joints): uncut spots that hold a part in the sheet.
  import Chips from '../../../components/params/Chips.svelte';
  import Placed from '../../../components/params/Placed.svelte';
  import Stepper from '../../../components/params/Stepper.svelte';
  import Toggle from '../../../components/params/Toggle.svelte';
  import { defaultJoints } from '../../../lib/features';
  import type { Features } from '../../../api';
  import { askPercentages, spots, type PickFeature, type SetFeatures } from './form';

  let { features, on, tab, contours, set, pick }: {
    features: Features;
    on: boolean;
    /** 0 shows the basic settings, 1 the rest. */
    tab: number;
    /** How many contours the drawing has. */
    contours: number;
    set: SetFeatures;
    pick: (feature: PickFeature) => void;
  } = $props();

  const PLACEMENTS = [['count', 'Per contour'], ['spacing', 'By spacing'], ['across_x', 'Across X'], ['across_y', 'Across Y'], ['manual', 'Picked']] as const;
  const BEHAVIOURS = [['laser_off', 'Laser off'], ['power', 'Low power']] as const;

  const joints = $derived(features.joints ?? defaultJoints());
  const placement = $derived(joints.placement);
  const kind = $derived(
    'count' in placement ? 'count'
      : 'spacing' in placement ? 'spacing'
      : 'across_x' in placement ? 'across_x'
      : 'across_y' in placement ? 'across_y'
      : 'manual');

  function setKind(v: string): void {
    set((f) => {
      f.joints!.placement = v === 'count' ? { count: 2 }
        : v === 'spacing' ? { spacing: 100 }
        : v === 'across_x' ? { across_x: 2 }
        : v === 'across_y' ? { across_y: 2 }
        : { manual: spots(f.joints!.placement) };
    });
  }
  const setCount = (v: number) => set((f) => { f.joints!.placement = { count: Math.max(1, Math.round(v)) }; });
  const setSpacing = (v: number) => set((f) => { f.joints!.placement = { spacing: v }; });
  const setAcrossX = (v: number) => set((f) => { f.joints!.placement = { across_x: Math.max(1, Math.round(v)) }; });
  const setAcrossY = (v: number) => set((f) => { f.joints!.placement = { across_y: Math.max(1, Math.round(v)) }; });
  const clearPicked = () => set((f) => { f.joints!.placement = { manual: [] }; });
  const setWidth = (v: number) => set((f) => { f.joints!.width = v; });
  const setMinimumSize = (v: number) => set((f) => { f.joints!.minimum_size = v; });
  const setOuterOnly = (v: boolean) => set((f) => { f.joints!.outer_only = v; });
  const setOpenStart = (v: boolean) => set((f) => { f.joints!.open_start = v; });
  const setBehaviour = (v: string) => set((f) => { f.joints!.behaviour = v === 'laser_off' ? 'laser_off' : { power: 10 }; });
  const setPower = (v: number) => set((f) => { f.joints!.behaviour = { power: Math.min(100, v) }; });
  const setSlow = (v: boolean) => set((f) => { f.joints!.slow_speed = v ? 5 : null; });
  const setSlowSpeed = (v: number) => set((f) => { f.joints!.slow_speed = Math.max(0.1, v); });
  const setRepierce = (v: boolean) => set((f) => { f.joints!.repierce = v; });
</script>

{#if tab === 0}
  <Chips label="Placement" options={PLACEMENTS} current={kind} disabled={!on} apply={setKind} />
  {#if 'count' in placement}
    <Stepper label="Joints per contour" value={placement.count} unit="" step={1} disabled={!on} apply={setCount} />
  {/if}
  {#if 'spacing' in placement}
    <Stepper label="Spacing" value={placement.spacing} unit="mm" step={10} disabled={!on} apply={setSpacing} />
  {/if}
  {#if 'across_x' in placement}
    <Stepper label="Grid lines across X" value={placement.across_x} unit="" step={1} disabled={!on} apply={setAcrossX} />
  {/if}
  {#if 'across_y' in placement}
    <Stepper label="Grid lines across Y" value={placement.across_y} unit="" step={1} disabled={!on} apply={setAcrossY} />
  {/if}
  {#if 'manual' in placement}
    <Placed count={placement.manual.length} what="joint" disabled={!on} onclear={clearPicked} onpick={() => pick('joints')} />
  {/if}
  <button class="btn btn-soft" onclick={() => askPercentages(features, 'joints', contours, set)} disabled={!on}>Enter positions (%)…</button>
  <Stepper label="Joint width" value={joints.width} unit="mm" step={0.1} disabled={!on} apply={setWidth} />
  <Stepper label="Skip contours under" value={joints.minimum_size} unit="mm" step={10} disabled={!on} apply={setMinimumSize} />
  <Toggle label="Outer contours only" value={joints.outer_only} disabled={!on} apply={setOuterOnly} />
{:else}
  <Toggle label="Joint at an open contour’s start" value={joints.open_start} disabled={!on} apply={setOpenStart} />
  <Chips label="In the joint" options={BEHAVIOURS} current={joints.behaviour === 'laser_off' ? 'laser_off' : 'power'} disabled={!on} apply={setBehaviour} />
  {#if typeof joints.behaviour !== 'string'}
    <Stepper label="Joint power" value={joints.behaviour.power} unit="%" step={5} disabled={!on} apply={setPower} />
  {/if}
  <Toggle label="Slow down in the joint" value={joints.slow_speed !== null} disabled={!on} apply={setSlow} />
  {#if joints.slow_speed !== null}
    <Stepper label="Joint speed" value={joints.slow_speed} unit="mm/s" step={1} disabled={!on} apply={setSlowSpeed} />
  {/if}
  <Toggle label="Pierce again after each joint" value={joints.repierce} disabled={!on} apply={setRepierce} />
  <p class="muted" style="font-size:var(--t-sm);margin:0">Piercing again needs a recipe with pierce stages.</p>
{/if}
