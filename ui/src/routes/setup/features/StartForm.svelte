<script lang="ts">
  // Where each contour's cut starts and which way it runs, and how its end
  // meets its start.
  import Chips from '../../../components/params/Chips.svelte';
  import Placed from '../../../components/params/Placed.svelte';
  import Stepper from '../../../components/params/Stepper.svelte';
  import { WORDS } from '../../../lib/features';
  import type { Features } from '../../../api';
  import type { PickFeature, SetFeatures } from './form';

  let { features, on, tab, set, pick }: {
    features: Features;
    on: boolean;
    /** 0 shows the start, 1 the seam. */
    tab: number;
    set: SetFeatures;
    pick: (feature: PickFeature) => void;
  } = $props();

  const POSITIONS = [['keep', WORDS['keep']!], ['automatic', WORDS['automatic']!], ['manual', 'Along each contour']] as const;
  const DIRECTIONS = [['keep', WORDS['keep']!], ['clockwise', 'Clockwise'], ['counterclockwise', 'Counterclockwise'], ['reverse', WORDS['reverse']!]] as const;
  const SEAMS = [['seal', 'Seal'], ['gap', 'Gap'], ['overcut', 'Overcut']] as const;

  const start = $derived(features.start);
  const seamKind = $derived(typeof features.seam === 'string' ? 'seal' : 'gap' in features.seam ? 'gap' : 'overcut');

  const setPosition = (v: string) => set((f) => { f.start.position = v === 'manual' ? { manual: 0.5 } : (v as 'keep' | 'automatic'); });
  const setAlong = (v: number) => set((f) => { f.start.position = { manual: Math.min(100, v) / 100 }; });
  const clearSpots = () => set((f) => { f.start.spots = []; });
  const setDirection = (v: string) => set((f) => { f.start.direction = v as Features['start']['direction']; });
  const setSeam = (v: string) => set((f) => { f.seam = v === 'seal' ? 'seal' : v === 'gap' ? { gap: 0.5 } : { overcut: 1 }; });
  const setSeamLength = (v: number) => set((f) => { f.seam = seamKind === 'gap' ? { gap: v } : { overcut: v }; });
</script>

{#if tab === 0}
  <Chips label="Start point" options={POSITIONS} current={typeof start.position === 'string' ? start.position : 'manual'} disabled={!on} apply={setPosition} />
  {#if typeof start.position !== 'string'}
    <Stepper label="Along the contour" value={Math.round(start.position.manual * 100)} unit="%" step={5} disabled={!on} apply={setAlong} />
  {/if}
  <Placed count={start.spots.length} what="chosen start" disabled={!on} onclear={clearSpots} onpick={() => pick('start')} />
  <Chips label="Direction" options={DIRECTIONS} current={start.direction} disabled={!on} apply={setDirection} />
{:else}
  <Chips label="Seam" options={SEAMS} current={seamKind} disabled={!on} apply={setSeam} />
  {#if typeof features.seam !== 'string'}
    {@const seam = features.seam}
    <Stepper label={'gap' in seam ? 'Gap' : 'Overcut'} value={'gap' in seam ? seam.gap : seam.overcut} unit="mm" step={0.1} disabled={!on} apply={setSeamLength} />
  {/if}
  <p class="muted" style="font-size:var(--t-sm);margin:0">Gap leaves an attachment; overcut cuts past the start.</p>
{/if}
