<script lang="ts">
  // The job's layers, as LightBurn's Cuts / Layers list has them: top to
  // bottom in the order they run, each with its colour, name, whether it is
  // output, whether it cuts through or marks the surface, and its recipe.
  // Drag the grip to reorder. With more than one layer to output, each needs
  // a recipe or to be switched off before the job compiles.
  import Modal from '../../components/Modal.svelte';
  import LayerRecipeChooser from './LayerRecipeChooser.svelte';
  import { api } from '../../api/client';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { explain, plural, recipeLabel } from '../../lib/format';
  import { PALETTE, layerColor } from '../../lib/drawing-layers';
  import { Reorder } from '../../lib/reorder.svelte';
  import type { DraftLayer, LayerChange, LayerMode } from '../../api';

  const draft = $derived(server.doc!.draft!);
  const layers = $derived(draft.layers);
  /** Several layers to output: each must be chosen. */
  const asking = $derived(layers.filter((l) => l.output).length > 1);
  const missing = $derived(asking && layers.some((l) => l.output && !l.chosen));
  let choosing = $state<DraftLayer | null>(null);
  let coloring = $state<DraftLayer | null>(null);
  let busy = $state(false);

  async function change(edit: LayerChange): Promise<void> {
    busy = true;
    try { await api.changeLayers(edit); } catch (error) { ui.say(explain(error), true); } finally { busy = false; }
  }
  function rename(layer: DraftLayer): void {
    osk.text('Layer name', layer.name, (to) => {
      if (to.trim() && to.trim() !== layer.name) void change({ kind: 'rename', from: layer.name, to });
    });
  }
  function color(layer: DraftLayer, value: string | null): void {
    coloring = null;
    const rgb = value ? ([1, 3, 5].map((i) => parseInt(value.slice(i, i + 2), 16)) as [number, number, number]) : null;
    void change({ kind: 'color', layer: layer.name, color: rgb });
  }
  /** What runs the layer, in a word or two. */
  function how(layer: DraftLayer): string {
    if (!layer.output) return 'Off';
    if (!layer.chosen) return asking ? 'Choose a recipe' : 'Job recipe';
    return layer.recipe ? recipeLabel(layer.recipe) : 'Job recipe';
  }

  // Dragging reorders a local list; the order is sent once, on release.
  let order = $state<string[]>([]);
  let grabbing = false;
  $effect(() => { order = layers.map((l) => l.name); });
  const rows = new Reorder({
    attribute: 'layer-row',
    order: () => order,
    save: (next) => { order = next; },
    editing: () => grabbing,
    horizontal: () => false,
  });
  function grab(e: PointerEvent): void { grabbing = true; rows.down(e); }
  function release(): void {
    if (!grabbing) return;
    grabbing = false;
    rows.up();
    if (order.join('\n') !== layers.map((l) => l.name).join('\n')) void change({ kind: 'order', layers: $state.snapshot(order) });
  }
  const shown = $derived(order.map((name) => layers.find((l) => l.name === name)).filter((l): l is DraftLayer => !!l));

  const hidden = (layer: string) => ui.hiddenDrawingLayers.includes(layer);
  function toggleHidden(layer: string): void {
    ui.hiddenDrawingLayers = hidden(layer)
      ? ui.hiddenDrawingLayers.filter((l) => l !== layer)
      : [...new Set([...ui.hiddenDrawingLayers, layer])];
  }
  const swatch = (layer: string) => layerColor(layers, layer) ?? 'var(--cut)';
  const setMode = (layer: DraftLayer, mode: LayerMode) => { if (layer.mode !== mode) void change({ kind: 'mode', layer: layer.name, mode }); };
</script>

<div class="feat-head">
  <h2>Layers</h2>
  <button class="btn btn-ghost" onclick={() => (ui.setupPanel = null)}>Close</button>
</div>
<p class="feat-desc">Run top to bottom. Drag the grip to reorder; select shapes and use Layer to move them.</p>
{#if missing}<p class="note warn-text">Choose a recipe or turn off each layer.</p>{/if}

<div class="layer-list" role="list">
  {#each shown as layer (layer.name)}
    <!-- The row holds the pointer while its grip drags it, so it hears the move and release. -->
    <div class="layer-row" data-layer-row={layer.name} class:placeholder={rows.dragging === layer.name} class:off={!layer.output}
      onpointermove={rows.move} onpointerup={release} onpointercancel={release} role="listitem">
      <div class="line">
        <button class="grip" aria-label="Drag {layer.name} up or down" onpointerdown={grab}><i class="ic ic-grip"></i></button>
        <button class="swatch-btn" aria-label="Colour of {layer.name}" disabled={busy} onclick={() => (coloring = layer)}>
          <span class="swatch" class:mark={layer.mode === 'mark'} style:--layer={swatch(layer.name)}></span>
        </button>
        <button class="name" disabled={busy} onclick={() => rename(layer)} aria-label="Rename {layer.name}">
          <strong>{layer.name}</strong>
          <small>{plural(layer.contours, 'shape')}{layer.machining ? ' · own machining' : ''}</small>
        </button>
        <button class="switch" class:on={layer.output} disabled={busy} aria-label="Output {layer.name}"
          onclick={() => change({ kind: 'output', layer: layer.name, on: !layer.output })}></button>
      </div>
      {#if layer.output}
        <div class="line settings">
          <div class="seg" role="group" aria-label="What {layer.name} does">
            <button class:on={layer.mode === 'cut'} disabled={busy} onclick={() => setMode(layer, 'cut')}>Cut</button>
            <button class:on={layer.mode === 'mark'} disabled={busy} onclick={() => setMode(layer, 'mark')}>Mark</button>
          </div>
          <button class="recipe" class:warn={asking && !layer.chosen} disabled={busy} onclick={() => (choosing = layer)}>{how(layer)}</button>
          <button class="eye" class:off={hidden(layer.name)} aria-label="Show or hide {layer.name}" onclick={() => toggleHidden(layer.name)}>
            <i class="ic {hidden(layer.name) ? 'ic-eye-off' : 'ic-eye'}"></i>
          </button>
        </div>
      {/if}
    </div>
  {/each}
</div>
<p class="muted legend">Cut goes through the sheet and makes the parts and holes. Mark traces the surface, without the job's machining.</p>

{#if choosing}<LayerRecipeChooser layer={choosing} onclose={() => (choosing = null)} />{/if}

{#if coloring}
  {@const layer = coloring}
  <Modal title="Colour of {layer.name}" onclose={() => (coloring = null)}>
    <div class="colors">
      {#each PALETTE as value (value)}
        <button class="color" style:--layer={value} aria-label="Colour {value}" onclick={() => color(layer, value)}></button>
      {/each}
    </div>
    <button class="btn btn-ghost block" onclick={() => color(layer, null)}>Colour from the file</button>
  </Modal>
{/if}

<style>
  .note { margin:0 0 10px; font-size:var(--t-sm); }
  .layer-list { display:grid; gap:8px; }
  .layer-row { display:grid; gap:6px; padding:6px; border:1px solid var(--line); border-radius:12px; background:var(--panel); }
  .layer-row.placeholder { opacity:.3; }
  .layer-row.off .name strong { color:var(--ink-3); }
  .line { display:flex; align-items:center; gap:6px; min-width:0; }
  .grip, .swatch-btn, .eye { flex:none; width:44px; height:44px; display:grid; place-items:center; border:0; border-radius:10px; background:transparent; color:var(--ink-3); cursor:pointer; }
  .grip { cursor:grab; touch-action:none; }
  .swatch { width:22px; height:22px; border-radius:6px; background:var(--layer); }
  .swatch.mark { background:transparent; border:3px dashed var(--layer); }
  .name { flex:1; min-width:0; min-height:44px; display:grid; gap:2px; padding:0 4px; border:0; background:transparent; color:var(--ink); text-align:left; cursor:pointer; }
  .name strong { font-size:var(--t-base); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .name small { font-size:var(--t-sm); color:var(--ink-3); }
  .settings { padding-left:50px; }
  .settings .seg button { min-height:44px; padding:0 12px; }
  .recipe { flex:1; min-width:0; min-height:44px; padding:0 12px; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); font:inherit; font-size:var(--t-sm); font-weight:600; text-align:left; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; cursor:pointer; }
  .recipe.warn { border-color:var(--warn); color:var(--warn); }
  .eye.off { opacity:.5; }
  .legend { margin:12px 0 0; font-size:var(--t-sm); line-height:1.5; }
  .colors { display:grid; grid-template-columns:repeat(5, 1fr); gap:10px; margin-bottom:12px; }
  .color { aspect-ratio:1; min-height:48px; border:2px solid var(--line); border-radius:12px; background:var(--layer); cursor:pointer; }
</style>
