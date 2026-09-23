<script lang="ts">
  // Bridges: channels that join two contours into one cut, or split one.
  import Placed from '../../../components/params/Placed.svelte';
  import Stepper from '../../../components/params/Stepper.svelte';
  import { defaultBridges } from '../../../lib/features';
  import type { Features } from '../../../api';
  import type { PickFeature, SetFeatures } from './form';

  let { features, on, set, pick }: {
    features: Features;
    on: boolean;
    set: SetFeatures;
    pick: (feature: PickFeature) => void;
  } = $props();

  const bridges = $derived(features.bridges ?? defaultBridges());
  const setWidth = (v: number) => set((f) => { f.bridges!.width = v; });
  const clear = () => set((f) => { f.bridges!.connections = []; });
  const undoLast = () => set((f) => { f.bridges!.connections.pop(); });
</script>

<Stepper label="Channel width" value={bridges.width} unit="mm" step={0.5} disabled={!on} apply={setWidth} />
<Placed count={bridges.connections.length} what="bridge" disabled={!on} onclear={clear} onpick={() => pick('bridges')} />
<div class="param">
  <div class="lbl">Tap two contours to join, or one contour twice to split.</div>
  <button class="btn btn-ghost" onclick={undoLast} disabled={!on || bridges.connections.length === 0}>Undo last</button>
</div>
