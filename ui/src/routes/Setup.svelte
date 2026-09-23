<script lang="ts">
  import SheetStrip from '../components/SheetStrip.svelte';
  import Canvas from './setup/Canvas.svelte';
  import Modal from '../components/Modal.svelte';
  import JobPanel from './setup/JobPanel.svelte';
  import FeaturePanel from './setup/FeaturePanel.svelte';
  import CopyPanel from './setup/CopyPanel.svelte';
  import ClipboardPanel from './setup/ClipboardPanel.svelte';
  import NestPanel from './setup/NestPanel.svelte';
  import { featureEdits } from '../stores/feature-edits';
  import { server } from '../stores/server.svelte';
  import { ui, type FeatureId } from '../stores/ui.svelte';
  import { explain } from '../lib/format';
  import { pasteable, type CopiedShapes, type PasteSettings } from '../lib/copy-paste';
  import { TOOLS, isOn, stateOf, toggled } from '../lib/features';
  import { flip } from 'svelte/animate';

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

  const EXTRA = [{ id: 'nest', short: 'Nest parts', name: 'Nest parts…' }, { id: 'copy', short: 'Copy job', name: 'Copy machining from job…' }];
  const tools = $derived(ui.favTools.filter((id) => TOOLS.some((t) => t.id === id) || EXTRA.some((t) => t.id === id)));
  let menu = $state(false);
  let selectedContours = $state<number[]>([]);
  let orderProgress = $state(0);
  let canvas = $state<{ paste: (count?: number) => Promise<void>; selectAll: () => void; selectPart: (first: number, count: number) => void }>();
  let clipboard = $state<CopiedShapes | null>(null);
  let pasting = $state(false);
  let pasteSettings = $state<PasteSettings>({ count: 1, gap: 10, direction: 'right' });

  // The L-shaped bar: the top row takes what fits, the rest goes down the rail.
  let head = $state<HTMLDivElement | null>(null);
  let cap = $state(6);
  $effect(() => {
    if (compact) { cap = tools.length; return; }
    if (!head) return;
    const measure = () => { cap = Math.max(1, Math.floor((head!.clientWidth - 110) / 126)); };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(head);
    return () => observer.disconnect();
  });

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

  // Bar editing, as the mockup does it: ✕ removes, the star in the menu
  // adds, and dragging reorders. A fixed ghost follows the pointer while
  // the real tile stays as a placeholder; the order changes only when the
  // pointer crosses into another slot, with a pause so the slide finishes.
  const EASE = 'cubic-bezier(.2,0,0,1)';
  let dragging = $state<string | null>(null);
  let drag: { id: string; ghost: HTMLElement; ox: number; oy: number; sx: number; sy: number; x: number; y: number; raf: number; moved: boolean; cool: number } | null = null;
  function barDown(e: PointerEvent): void {
    if (!ui.editBar || (e.target as HTMLElement).closest('[data-remove]')) return;
    const el = (e.target as HTMLElement).closest<HTMLElement>('.feature[data-feature]');
    if (!el) return;
    const r = el.getBoundingClientRect();
    const ghost = el.cloneNode(true) as HTMLElement;
    ghost.classList.add('ghost');
    ghost.classList.remove('editing', 'open', 'placeholder');
    ghost.querySelector('[data-remove]')?.remove();
    Object.assign(ghost.style, { position: 'fixed', left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px`, margin: '0', transform: 'none' });
    document.body.appendChild(ghost);
    drag = { id: el.dataset['feature']!, ghost, ox: e.clientX - r.left, oy: e.clientY - r.top, sx: e.clientX, sy: e.clientY, x: e.clientX, y: e.clientY, raf: 0, moved: false, cool: 0 };
    el.setPointerCapture(e.pointerId);
    e.preventDefault();
  }
  function tick(): void {
    if (!drag) return;
    drag.raf = 0;
    if (!drag.moved) { if (Math.hypot(drag.x - drag.sx, drag.y - drag.sy) < 6) return; drag.moved = true; dragging = drag.id; }
    drag.ghost.style.transform = `translate(${drag.x - drag.ox - parseFloat(drag.ghost.style.left)}px, ${drag.y - drag.oy - parseFloat(drag.ghost.style.top)}px)`;
    if (performance.now() < drag.cool) return;
    const hit = document.elementFromPoint(drag.x, drag.y)?.closest<HTMLElement>('.feature[data-feature]');
    const target = hit?.dataset['feature'];
    if (!hit || !target || target === drag.id) return;
    const tr = hit.getBoundingClientRect();
    const horizontal = !!hit.closest('.features');
    const after = horizontal ? drag.x > tr.left + tr.width / 2 : drag.y > tr.top + tr.height / 2;
    const order = ui.favTools.filter((x) => x !== drag!.id);
    order.splice(order.indexOf(target) + (after ? 1 : 0), 0, drag.id);
    if (order.join() === ui.favTools.join()) return;
    ui.setBar(order);
    drag.cool = performance.now() + 200;
    setTimeout(() => { if (drag && !drag.raf) drag.raf = requestAnimationFrame(tick); }, 210);
  }
  function barMove(e: PointerEvent): void {
    if (!drag) return;
    drag.x = e.clientX;
    drag.y = e.clientY;
    if (!drag.raf) drag.raf = requestAnimationFrame(tick);
  }
  function barUp(): void {
    if (!drag) return;
    const { id, ghost } = drag;
    drag = null;
    const r = document.querySelector<HTMLElement>(`.feature[data-feature="${id}"]`)?.getBoundingClientRect();
    const finish = () => { ghost.remove(); dragging = null; };
    if (!r) { finish(); return; }
    ghost.animate([{ transform: ghost.style.transform }, { transform: `translate(${r.left - parseFloat(ghost.style.left)}px, ${r.top - parseFloat(ghost.style.top)}px)` }], { duration: 180, easing: EASE }).onfinish = finish;
  }

  const tile = (id: string) => TOOLS.find((t) => t.id === id) ?? null;

</script>

{#snippet feature(id: string, t: { short: string } | null)}
  <button class="feature" data-feature={id} class:editing={ui.editBar} class:set={t && draft ? isOn(draft.features, id) : false} class:open={ui.setupPanel === id} class:tool={!t} class:placeholder={dragging === id}
    onclick={() => { if (!ui.editBar) runTool(id); }} onpointerdown={barDown} onpointermove={barMove} onpointerup={barUp} onpointercancel={barUp}>
    {#if ui.editBar}<span class="remove" data-remove onclick={(e) => { e.stopPropagation(); ui.setBar(ui.favTools.filter((x) => x !== id)); }} role="button" tabindex="-1" onkeydown={() => undefined}><i class="ic ic-x"></i></span>{/if}
    <span>{t ? t.short : EXTRA.find((x) => x.id === id)?.short}</span><span class="state">{t && draft ? stateOf(draft.features, id) : 'tool'}</span>
  </button>
{/snippet}

<section class="panel main canvas-panel" bind:this={canvasPanel}>
  <div class="setup-head" bind:this={head}>
    <div class="features" role="toolbar" tabindex="-1" aria-label="Machining tools">
      {#each tools.slice(0, cap) as id (id)}
        {@const t = tile(id)}
        <div class="slot" animate:flip={{ duration: 180 }}>{@render feature(id, t)}</div>
      {/each}
      {#if ui.editBar}<button class="feature done-btn" onclick={() => (ui.editBar = false)}><span>Done</span><span class="state">editing bar</span></button>{/if}
    </div>
    <div class="all-tools">
      <button class="feature all-btn" onclick={() => (menu = !menu)} hidden={ui.editBar}><span>All tools</span><span class="state">browse</span></button>

    </div>
  </div>

  <SheetStrip />
  <Canvas bind:this={canvas} bind:selectedContours bind:clipboard bind:pasting {pasteSettings} {orderProgress}>
    {#snippet rail()}
      {#each tools.slice(cap) as id (id)}
        {@const t = tile(id)}
        <div class="slot" animate:flip={{ duration: 180 }}>{@render feature(id, t)}</div>
      {/each}
    {/snippet}
  </Canvas>
</section>

<aside class="panel side" class:spread={ui.setupPanel === null} bind:this={settingsPanel}>
  {#if !draft}
    <h2>No part open</h2>
    <p class="muted">Pick a part or a saved job on the Parts page.</p>
    <div class="side-foot"><button class="btn btn-primary lg block" onclick={() => (ui.tab = 'parts')}>Go to Parts</button></div>
  {:else if ui.setupPanel === 'copy'}
    <CopyPanel />
  {:else if ui.setupPanel === 'clipboard'}
    <ClipboardPanel {clipboard} bind:settings={pasteSettings} {pasting} disabled={draft.error === 'preparing geometry' || !pasteable(clipboard, draft) || !!ui.picking || ui.nestShown || ui.nestPicking} onpaste={() => { void canvas?.paste(pasteSettings.count); }} />
  {:else if ui.setupPanel === 'nest'}
    <NestPanel {selectedContours} onselectall={() => canvas?.selectAll()} />
  {:else if ui.setupPanel}
    <FeaturePanel id={ui.setupPanel} {selectedContours} {compact} bind:orderProgress ontoggle={() => toggle(ui.setupPanel!)} />
  {:else}
    <JobPanel onselectpart={(first, count) => canvas?.selectPart(first, count)} />
  {/if}
</aside>


{#if menu}
  <Modal title="All tools" onclose={() => (menu = false)}>
    <div class="tools-heading"><span>Starred tools are on the bar</span><button class="btn btn-ghost" onclick={() => { ui.editBar = true; menu = false; }}>Edit bar</button></div>
    <div class="tool-choices">
      {#each [...TOOLS.map(t => ({ id: t.id, name: t.name, meta: draft ? stateOf(draft.features, t.id) : '' })), ...EXTRA.map(t => ({ id: t.id, name: t.name, meta: '' }))] as t}
        {@const starred = ui.favTools.includes(t.id)}
        <div class="tool-choice">
          <button class="tool-star" class:on={starred} aria-label="{starred ? 'Unstar' : 'Star'} {t.name}" aria-pressed={starred} onclick={() => ui.setBar(starred ? ui.favTools.filter(x => x !== t.id) : [...ui.favTools, t.id])}><i class="ic {starred ? 'ic-star-fill' : 'ic-star'}"></i></button>
          <button class="tool-open" onclick={() => { menu = false; runTool(t.id); }}><span>{t.name}</span><small>{t.meta}</small></button>
        </div>
      {/each}
    </div>
  </Modal>
{/if}

<style>
  .tools-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 14px; color: var(--ink-3); font-size: var(--t-sm); }
  .tool-choices { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
  .tool-choice { display: grid; grid-template-columns: 44px 1fr; min-width: 0; border: 1px solid var(--line); border-radius: 10px; background: var(--panel-2); }
  .tool-star { width: 44px; min-height: 58px; border: 0; border-radius: 10px 0 0 10px; background: transparent; color: var(--ink-3); font-size: var(--t-lg); cursor: pointer; }
  .tool-star.on { color: var(--warn); }
  .tool-open { display: flex; flex-direction: column; justify-content: center; align-items: start; gap: 4px; min-width: 0; min-height: 58px; padding: 8px 10px 8px 0; border: 0; border-radius: 0 10px 10px 0; background: transparent; color: var(--ink); font: inherit; font-size: var(--t-sm); font-weight: 600; text-align: left; cursor: pointer; }
  .tool-open small { color: var(--ink-3); font-size: var(--t-sm); font-weight: 400; }
  .tool-star:active, .tool-open:active { background: var(--accent-soft); }
  @media (max-width: 520px) { .tool-choices { grid-template-columns: 1fr; } }
</style>
