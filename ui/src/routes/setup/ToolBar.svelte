<script lang="ts" module>
  /** One tool on a bar. */
  export type Tool = {
    label: string;
    title?: string;
    when: 'idle' | 'selected' | 'always';
    icon?: string;
    /** Shown instead of the icon, such as a value. */
    text?: () => string;
    /** A second line under the label, such as a setting's state. */
    detail?: () => string;
    /** Its panel is open, or it is switched on. */
    on?: () => boolean;
    /** Its feature is in use. */
    set?: () => boolean;
    disabled: () => boolean;
    run: () => void;
    /** Off the bar while it cannot apply at all. */
    absent?: () => boolean;
  };
</script>

<script lang="ts">
  // One row of tools, shared by Setup's machining bar and the drawing bar:
  // Order lets the operator drag the tools into their own order, the same
  // way on both. `icons` tiles show an icon over the label; `names` tiles
  // show the label over the tool's state. A `corner` bar shows every tool:
  // those that do not fit along the top continue down a column, placed by
  // the parent grid's `top` and `side` areas.
  import { flip } from 'svelte/animate';
  import { Reorder } from '../../lib/reorder.svelte';

  let { tools, order, save, editing = $bindable(false), selected = false, variant = 'icons', label, class: className = '', corner = false }: {
    tools: Record<string, Tool>;
    /** Every tool's place, the ones not shown too. */
    order: string[];
    save: (order: string[]) => void;
    editing?: boolean;
    /** Something is selected: `selected` tools show, `idle` ones hide. */
    selected?: boolean;
    variant?: 'icons' | 'names';
    label: string;
    class?: string;
    /** Every tool shown: the row, then a column down the side. */
    corner?: boolean;
  } = $props();

  // While arranging, every tool shows so each can be placed.
  const shows = (tool: Tool): boolean =>
    !tool.absent?.() && (tool.when === 'always' || tool.when === (selected ? 'selected' : 'idle'));
  const shown = $derived(order.filter((id) => {
    const tool = tools[id];
    return !!tool && (editing || shows(tool));
  }));
  // Tiles fill the top row, as many as fit at their least width; the rest go down the side.
  const TILE = { icons: 56, names: 136 } as const;
  const GAP = 6;
  let row = $state<HTMLDivElement | null>(null);
  let width = $state(0);
  $effect(() => {
    if (!row || !corner) return;
    const observer = new ResizeObserver(([entry]) => { width = entry?.contentRect.width ?? 0; });
    observer.observe(row);
    return () => observer.disconnect();
  });
  const fits = $derived(corner && width > 0 ? Math.max(1, Math.floor((width + GAP) / (TILE[variant] + GAP))) : Infinity);
  const top = $derived(shown.slice(0, fits));
  const side = $derived(corner ? shown.slice(fits) : []);
  const bar = new Reorder({
    attribute: 'bar-tool', order: () => order, save: (next) => save(next), editing: () => editing,
    horizontal: (tile) => !tile.closest('.tool-col'),
  });
  function run(tool: Tool): void {
    if (!editing) tool.run();
  }
</script>

{#snippet tile(id: string)}
  {@const tool = tools[id]!}
  <button class="tile" data-bar-tool={id} class:on={tool.on?.() ?? false} class:set={tool.set?.() ?? false}
    class:placeholder={bar.dragging === id} class:editing
    title={tool.title ?? tool.label} disabled={!editing && tool.disabled()} onclick={() => run(tool)}
    onpointerdown={bar.down} onpointermove={bar.move} onpointerup={bar.up} onpointercancel={bar.up}>
    {#if variant === 'names'}
      {#if tool.icon}<i class="ic {tool.icon}"></i>{/if}
      <span class="texts"><span class="name">{tool.label}</span>{#if tool.detail}<small class="detail">{tool.detail()}</small>{/if}</span>
    {:else}
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
    {/if}
  </button>
{/snippet}

<div class="tool-bar {variant} {className}" class:corner role="toolbar" aria-label={label}>
  <div class="tool-row" class:editing>
    <!-- The tools scroll; Order stays in reach at the end. -->
    <div class="tiles" bind:this={row}>
      {#each top as id (id)}
        <div class="slot" animate:flip={{ duration: 180 }}>{@render tile(id)}</div>
      {/each}
    </div>
    {#if editing || !selected}
      <button class="tile arrange" class:on={editing}
        title={editing ? 'Finish arranging' : 'Drag the tools into your order'} onclick={() => (editing = !editing)}>
        <i class="ic {editing ? 'ic-check' : 'ic-grip'}"></i><small>{editing ? 'Done' : 'Order'}</small>
      </button>
    {/if}
  </div>
  {#if side.length}
    <div class="tool-col" class:editing>
      {#each side as id (id)}
        <div class="slot" animate:flip={{ duration: 180 }}>{@render tile(id)}</div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .tool-bar { flex-shrink: 0; padding: 8px 12px; background: var(--panel); }
  .tool-row { display: flex; gap: 6px; align-items: stretch; }
  .tiles { flex: 1; min-width: 0; display: flex; gap: 6px; overflow-x: auto; scrollbar-width: none; }
  .tiles > .slot { flex: 1 0 56px; min-width: 56px; }
  .names .tiles > .slot { flex: 1 0 136px; min-width: 136px; }
  .editing .tiles { flex-wrap: wrap; overflow: visible; }
  .editing .tiles > .slot { flex: 0 0 72px; }
  .names .editing .tiles > .slot { flex: 0 0 136px; }
  .tool-row > .arrange { flex: 0 0 56px; align-self: flex-start; }
  .editing .tiles .tile, .tool-col.editing .tile { cursor: grab; border-style: dashed; touch-action: none; }
  /* A corner bar lays its row and column into the parent's grid. */
  .tool-bar.corner { display: contents; }
  .corner > .tool-row { grid-area: top; padding: 8px 12px; background: var(--panel); border-bottom: 1px solid var(--line); }
  .corner .editing .tiles { flex-wrap: nowrap; overflow: hidden; }
  .corner .editing .tiles > .slot { flex: 1 0 136px; }
  .tool-col { grid-area: side; width: 160px; display: flex; flex-direction: column; gap: 6px; padding: 8px; overflow-y: auto; scrollbar-width: none; background: var(--panel); border-right: 1px solid var(--line); }
  .tool-col > .slot { flex: none; }
  .tile {
    width: 100%; min-height: 66px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 7px;
    border-radius: 12px; border: 1px solid var(--line); background: var(--panel-2); color: var(--ink); font: inherit; font-weight: 700; line-height: 1; cursor: pointer;
  }
  .names .tile { min-height: 48px; flex-direction: row; align-items: center; justify-content: flex-start; gap: 8px; padding: 4px 10px; text-align: left; }
  .names .tile .ic { width: 20px; height: 20px; flex: none; }
  .texts { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .tile small { font-size: var(--t-sm); font-weight: 700; color: var(--ink-3); white-space: nowrap; letter-spacing: 0; }
  .tile strong { font-size: var(--t-base); }
  .tile .name { font-size: var(--t-base); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 100%; }
  .tile .detail { font-weight: 600; max-width: 100%; overflow: hidden; text-overflow: ellipsis; }
  .tile .ic { width: 23px; height: 23px; }
  .tile.arrange { align-items: center; }
  .tile.on { background: var(--accent-soft); border-color: transparent; color: var(--accent-2); }
  .tile.on small { color: var(--accent-2); }
  .tile.set .detail { color: var(--accent-2); }
  .tile.placeholder { opacity: .3; }
  .tile:disabled { opacity: .38; cursor: default; }
</style>
