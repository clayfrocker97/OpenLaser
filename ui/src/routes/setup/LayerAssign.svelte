<script lang="ts">
  // Where selected shapes go, as LightBurn's palette sends them: a layer the
  // job has, or a new one named here, whose settings open next.
  import Modal from '../../components/Modal.svelte';
  import { api } from '../../api/client';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { explain, plural } from '../../lib/format';
  import { swatchColor } from '../../lib/drawing-layers';
  import type { DraftLayer } from '../../api';

  let { contours, layers, onclose }: { contours: number[]; layers: DraftLayer[]; onclose: () => void } = $props();
  let busy = $state(false);

  /** A new layer opens its settings next. */
  async function assign(layer: string, fresh = false): Promise<void> {
    busy = true;
    try {
      await api.changeLayers({ kind: 'assign', contours, layer });
      ui.say(`${plural(contours.length, 'shape')} on ${layer} · Undo moves them back`);
      onclose();
      if (fresh) ui.layerSheet = layer;
    } catch (error) { ui.say(explain(error), true); } finally { busy = false; }
  }
  function create(): void {
    osk.text('New layer name', '', (name) => { if (name.trim()) void assign(name.trim(), !layers.some((l) => l.name === name.trim())); });
  }
</script>

<Modal title="Move {plural(contours.length, 'shape')} to" onclose={() => { if (!busy) onclose(); }}>
  <div class="targets">
    {#each layers as layer (layer.name)}
      <button class="chip" disabled={busy} onclick={() => assign(layer.name)}>
        <span class="swatch" style:--layer={swatchColor(layers, layer.name)}></span>{layer.name}
      </button>
    {/each}
    <button class="chip new" disabled={busy} onclick={create}><i class="ic ic-plus"></i>New layer…</button>
  </div>
</Modal>

<style>
  .targets { display:flex; flex-wrap:wrap; gap:8px; }
  .chip { min-height:48px; display:inline-flex; align-items:center; gap:8px; }
  .swatch { width:16px; height:16px; border-radius:5px; background:var(--layer); }
  .new { display:inline-flex; align-items:center; gap:6px; }
</style>
