<script lang="ts">
  // Where selected shapes go: a layer the job has, or a new one named here.
  import Modal from '../../components/Modal.svelte';
  import { api } from '../../api/client';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { explain, plural } from '../../lib/format';
  import { layerColor } from '../../lib/drawing-layers';
  import type { DraftLayer } from '../../api';

  let { contours, layers, onclose }: { contours: number[]; layers: DraftLayer[]; onclose: () => void } = $props();
  let busy = $state(false);

  async function assign(layer: string): Promise<void> {
    busy = true;
    try {
      await api.changeLayers({ kind: 'assign', contours, layer });
      ui.say(`${plural(contours.length, 'shape')} on ${layer} · Undo moves them back`);
      onclose();
    } catch (error) { ui.say(explain(error), true); } finally { busy = false; }
  }
  function create(): void {
    osk.text('New layer name', '', (name) => { if (name.trim()) void assign(name.trim()); });
  }
</script>

<Modal title="Move {plural(contours.length, 'shape')} to" onclose={() => { if (!busy) onclose(); }}>
  <div class="targets">
    {#each layers as layer (layer.name)}
      <button class="chip" disabled={busy} onclick={() => assign(layer.name)}>
        <span class="swatch" style:--layer={layerColor(layers, layer.name) ?? 'var(--cut)'}></span>{layer.name}
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
