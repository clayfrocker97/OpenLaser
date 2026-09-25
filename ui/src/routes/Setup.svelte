<script lang="ts">
  import SheetStrip from '../components/SheetStrip.svelte';
  import Canvas from './setup/Canvas.svelte';
  import JobPanel from './setup/JobPanel.svelte';
  import FeaturePanel from './setup/FeaturePanel.svelte';
  import LayerSheet from './setup/LayerSheet.svelte';
  import CopyPanel from './setup/CopyPanel.svelte';
  import ClipboardPanel from './setup/ClipboardPanel.svelte';
  import NestPanel from './setup/NestPanel.svelte';
  import CreateText from '../components/CreateText.svelte';
  import { featureEdits } from '../stores/feature-edits';
  import { server } from '../stores/server.svelte';
  import { ui, type FeatureId } from '../stores/ui.svelte';
  import { explain } from '../lib/format';
  import { pasteable, type CopiedShapes, type PasteSettings } from '../lib/copy-paste';
  import { TOOLS, isOn, stateOf, toggled } from '../lib/features';
  import ToolBar, { type Tool } from './setup/ToolBar.svelte';

  let { compact = false }: { compact?: boolean } = $props();
  let canvasPanel = $state<HTMLElement | null>(null);
  let settingsPanel = $state<HTMLElement | null>(null);
  const picking = $derived(!!ui.picking || ui.nestPicking);
  $effect(() => {
    if (compact && ui.setupPanel) settingsPanel?.scrollIntoView({ block: 'start', behavior: 'smooth' });
  });
  $effect(() => {
    if (compact && picking) canvasPanel?.scrollIntoView({ block: 'start', behavior: 'smooth' });
  });
  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft);

  const EXTRA = [{ id: 'nest', short: 'Nest parts' }, { id: 'copy', short: 'Copy job' }];
  let texting = $state(false);
  let selectedContours = $state<number[]>([]);
  let orderProgress = $state(0);
  let canvas = $state<{ paste: (count?: number) => Promise<void>; selectAll: () => void; selectPart: (first: number, count: number) => void }>();
  let clipboard = $state<CopiedShapes | null>(null);
  let pasting = $state(false);
  let pasteSettings = $state<PasteSettings>({ count: 1, gap: 10, direction: 'right' });

  function runTool(id: string): void {
    if (id === 'nest') { ui.setupPanel = ui.setupPanel === 'nest' ? null : 'nest'; return; }
    if (id === 'copy') { ui.setupPanel = ui.setupPanel === 'copy' ? null : 'copy'; return; }
    ui.setupPanel = ui.setupPanel === id ? null : (id as FeatureId);
  }

  async function toggle(id: string): Promise<void> {
    if (!draft) return;
    if (id === 'common' && !draft.features.common && selectedContours.length < 2) {
      ui.say('Select at least two adjacent contours on the drawing.', true);
      return;
    }
    try {
      await featureEdits.change((features) => Object.assign(features, toggled(features, id, selectedContours)));
    } catch (error) {
      ui.say(explain(error), true);
    }
  }

  /** The machining bar's tools, laid out and ordered by the same ToolBar as the drawing bar. */
  const BAR_TOOLS = $derived(Object.fromEntries([
    ...TOOLS.map((t): [string, Tool] => [t.id, {
      label: t.short, when: 'always', icon: `ic-tool-${t.id}`, detail: () => (draft ? stateOf(draft.features, t.id) : ''),
      on: () => ui.setupPanel === t.id, set: () => !!draft && isOn(draft.features, t.id), disabled: () => false, run: () => runTool(t.id),
    }]),
    ...EXTRA.map((t): [string, Tool] => [t.id, {
      label: t.short, when: 'always', icon: t.id === 'nest' ? 'ic-tool-nest' : 'ic-copy', detail: () => 'tool', on: () => ui.setupPanel === t.id, disabled: () => false, run: () => runTool(t.id),
    }]),
  ]));

</script>

<section class="panel main canvas-panel" class:corner={!compact} bind:this={canvasPanel}>
  <ToolBar class="setup-head" variant="names" label="Machining tools" tools={BAR_TOOLS} order={ui.bar} save={(order) => ui.setBar(order)}
    bind:editing={ui.editBar} corner={!compact} />

  <SheetStrip />
  <Canvas bind:this={canvas} bind:selectedContours bind:clipboard bind:pasting {pasteSettings} {orderProgress} />
</section>

<aside class="panel side" class:spread={ui.setupPanel === null} bind:this={settingsPanel}>
  {#if !draft}
    <h2>No part open</h2>
    <p class="muted">Pick a part or a saved job on the Parts page, or start from text.</p>
    <div class="side-foot"><button class="btn btn-ghost lg block" onclick={() => (texting = true)}>Add text</button><button class="btn btn-primary lg block" onclick={() => (ui.tab = 'parts')}>Go to Parts</button></div>
  {:else if ui.setupPanel === 'copy'}
    <CopyPanel />
  {:else if ui.setupPanel === 'clipboard'}
    <ClipboardPanel
      {clipboard} bind:settings={pasteSettings} {pasting}
      disabled={draft.error === 'preparing geometry' || !pasteable(clipboard, draft) || !!ui.picking || ui.nestShown || ui.nestPicking}
      onpaste={() => { void canvas?.paste(pasteSettings.count); }} />
  {:else if ui.setupPanel === 'nest'}
    <NestPanel {selectedContours} onselectall={() => canvas?.selectAll()} />
  {:else if ui.setupPanel}
    <FeaturePanel id={ui.setupPanel} {selectedContours} {compact} bind:orderProgress ontoggle={() => toggle(ui.setupPanel!)} />
  {:else}
    <JobPanel onselectpart={(first, count) => canvas?.selectPart(first, count)} />
  {/if}
</aside>
{#if texting}<CreateText onclose={() => (texting = false)} />{/if}
{#if ui.layerSheet && draft}{#key ui.layerSheet}<LayerSheet name={ui.layerSheet} onclose={() => (ui.layerSheet = null)} />{/key}{/if}



<style>
  /* The machining bar runs along the top and down the left; the drawing bar spans the foot. */
  .canvas-panel.corner {
    display: grid; grid-template-columns: auto minmax(0, 1fr); grid-template-rows: auto auto minmax(0, 1fr) auto;
    grid-template-areas: 'top top' 'side strip' 'side canvas' 'bottom bottom';
  }
  .canvas-panel.corner > :global(.sheet-strip) { grid-area: strip; }
  .canvas-panel.corner > :global(.canvas-wrap) { grid-area: canvas; }
  .canvas-panel.corner > :global(.drawing-toolbar) { grid-area: bottom; }
</style>
