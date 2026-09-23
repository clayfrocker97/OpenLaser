<script lang="ts" module>
  /** One tool on the drawing bar. */
  export type DrawTool = {
    label: string;
    title?: string;
    when: 'idle' | 'selected' | 'always';
    icon?: string;
    text?: () => string;
    on?: () => boolean;
    disabled: () => boolean;
    run: () => void;
    /** Off the bar while it cannot apply at all. */
    absent?: () => boolean;
  };
</script>

<script lang="ts">
  // The drawing bar is one row. Editing tools appear only while something is
  // selected; the view tools only while nothing is. Undo, Redo and Paste stay.
  // Arrange lets the operator drag the tools into their own order.
  import { flip } from 'svelte/animate';
  import { Reorder } from '../../lib/reorder.svelte';
  import { ui } from '../../stores/ui.svelte';

  let { tools, selected }: { tools: Record<string, DrawTool>; selected: boolean } = $props();

  // While arranging, every tool shows so each can be placed.
  const shows = (tool: DrawTool): boolean =>
    !tool.absent?.() && (tool.when === 'always' || tool.when === (selected ? 'selected' : 'idle'));
  const order = $derived(ui.drawBar.filter((id) => {
    const tool = tools[id];
    if (!tool) return false;
    return ui.editDrawBar || shows(tool);
  }));
  const bar = new Reorder({
    attribute: 'draw-tool',
    order: () => ui.drawBar,
    save: (order) => ui.setDrawBar(order),
    editing: () => ui.editDrawBar,
  });
  function run(tool: DrawTool): void {
    if (!ui.editDrawBar) tool.run();
  }
  function arrange(): void {
    ui.editDrawBar = !ui.editDrawBar;
  }
</script>

<div class="drawing-toolbar" role="toolbar" aria-label="Drawing tools">
  <div class="tool-row" class:editing={ui.editDrawBar}>
    {#each order as id (id)}
      {@const tool = tools[id]!}
      <div class="slot" animate:flip={{ duration: 180 }}>
        <button class="rail-btn" data-draw-tool={id} class:on={tool.on?.() ?? false}
          class:placeholder={bar.dragging === id} class:editing={ui.editDrawBar}
          title={tool.title ?? tool.label} disabled={!ui.editDrawBar && tool.disabled()} onclick={() => run(tool)}
          onpointerdown={bar.down} onpointermove={bar.move} onpointerup={bar.up} onpointercancel={bar.up}>
          {#if id === 'group'}
            <svg width="23" height="23" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
              <rect x="2" y="2" width="20" height="20" rx="2" stroke-dasharray="3 2"/>
              <rect x="5" y="5" width="7" height="7"/><rect x="12" y="12" width="7" height="7"/>
            </svg>
          {:else if id === 'ungroup'}
            <svg width="23" height="23" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
              <rect x="2" y="2" width="8" height="8"/><rect x="14" y="14" width="8" height="8"/>
              <path d="M14 4h6v6M20 4l-7 7M4 14v6h6M4 20l7-7"/>
            </svg>
          {:else if tool.text}<strong>{tool.text()}</strong>
          {:else}<i class="ic {tool.icon}"></i>{/if}
          <small>{tool.label}</small>
        </button>
      </div>
    {/each}
    {#if ui.editDrawBar || !selected}
      <button class="rail-btn arrange" class:on={ui.editDrawBar}
        title={ui.editDrawBar ? 'Finish arranging' : 'Drag the tools into your order'} onclick={arrange}>
        <i class="ic {ui.editDrawBar ? 'ic-check' : 'ic-grip'}"></i><small>{ui.editDrawBar ? 'Done' : 'Order'}</small>
      </button>
    {/if}
  </div>
</div>

<style>
  .drawing-toolbar { flex-shrink: 0; display: grid; gap: 8px; padding: 12px; border-top: 1px solid var(--line); background: var(--panel); overflow-x: auto; }
  .tool-row { display: flex; gap: 6px; }
  .tool-row > .slot { flex: 1 0 56px; min-width: 56px; }
  .tool-row.editing { flex-wrap: wrap; }
  .tool-row.editing > .slot { flex: 0 0 72px; }
  .tool-row > .arrange { flex: 0 0 52px; margin-left: auto; }
  .tool-row small { white-space: nowrap; }
  .tool-row.editing .rail-btn:not(.arrange) { cursor: grab; border-style: dashed; touch-action: none; }
  .rail-btn.placeholder { opacity: .3; }
  .rail-btn { min-height: 66px; width: 100%; gap: 7px; }
  .rail-btn strong { font-size: var(--t-base); }
  .rail-btn small { font-size: var(--t-sm); letter-spacing: 0; }
  .rail-btn:disabled { opacity: .38; cursor: default; }
  .rail-btn .ic { width: 23px; height: 23px; }
  .vertical { transform: rotate(90deg); }
</style>
