<script lang="ts">
  // The drawing's layers: shown or hidden here, cut or skipped in the job.
  import { featureEdits } from '../../stores/feature-edits';
  import { ui } from '../../stores/ui.svelte';
  import { explain, plural } from '../../lib/format';
  import type { LayerView } from '../../api';

  let { layers, skipped, onclose }: {
    layers: LayerView[];
    /** The layers the job skips. */
    skipped: string[];
    onclose: () => void;
  } = $props();

  function skip(layer: string, skip: boolean): void {
    featureEdits.change((f) => {
      f.skip_layers = skip ? [...new Set([...f.skip_layers, layer])] : f.skip_layers.filter((l) => l !== layer);
    }).catch((error: unknown) => ui.say(explain(error), true));
  }
  const hidden = (layer: string) => ui.hiddenDrawingLayers.includes(layer);
  function toggleHidden(layer: string): void {
    const off = !hidden(layer);
    ui.hiddenDrawingLayers = off
      ? [...new Set([...ui.hiddenDrawingLayers, layer])]
      : ui.hiddenDrawingLayers.filter((l) => l !== layer);
  }
</script>

<div class="layers-pop">
  <div class="lp-head"><h3>Layers</h3><button class="link" onclick={onclose}>Close</button></div>
  {#each layers as layer (layer.name)}
    {@const isSkipped = skipped.includes(layer.name)}
    <div class="lrow" class:hidden-layer={hidden(layer.name)}>
      <button class="eye" class:off={hidden(layer.name)} title="Show or hide" onclick={() => toggleHidden(layer.name)}>
        <span class="sw" style="background:{isSkipped ? 'var(--ink-3)' : 'var(--ink)'}"></span>
      </button>
      <div>
        <div class="lname">{layer.name}</div>
        <div class="lmeta">{plural(layer.contours, 'contour')} · {isSkipped ? 'Skipped' : 'Cut'}</div>
      </div>
      <div class="seg">
        <button class:on={!isSkipped} onclick={() => skip(layer.name, false)}>Cut</button>
        <button class:on={isSkipped} onclick={() => skip(layer.name, true)}>Skip</button>
      </div>
    </div>
  {/each}
</div>
