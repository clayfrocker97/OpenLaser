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
  // show the label over the tool's state.
  import { flip } from 'svelte/animate';
  import { Reorder } from '../../lib/reorder.svelte';

  let { tools, order, save, editing = $bindable(false), selected = false, variant = 'icons', label, class: className = '', more }: {
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
    /** A tile before Order that opens every tool, such as a menu. */
    more?: { label: string; detail: string; run: () => void };
  } = $props();

  // While arranging, every tool shows so each can be placed.
  const shows = (tool: Tool): boolean =>
    !tool.absent?.() && (tool.when === 'always' || tool.when === (selected ? 'selected' : 'idle'));
  const shown = $derived(order.filter((id) => {
    const tool = tools[id];
    return !!tool && (editing || shows(tool));
  }));
  const bar = new Reorder({ attribute: 'bar-tool', order: () => order, save: (next) => save(next), editing: () => editing });
  function run(tool: Tool): void {
    if (!editing) tool.run();
  }
</script>

<div class="tool-bar {variant} {className}" role="toolbar" aria-label={label}>
  <div class="tool-row" class:editing>
    <!-- The tools scroll; All tools and Order stay in reach at the end. -->
    <div class="tiles">
    {#each shown as id (id)}
      {@const tool = tools[id]!}
      <div class="slot" animate:flip={{ duration: 180 }}>
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
      </div>
    {/each}
    </div>
    {#if more && !editing}
      <button class="tile more" onclick={more.run}>
        {#if variant === 'names'}<span class="texts"><span class="name">{more.label}</span><small class="detail">{more.detail}</small></span>
        {:else}<i class="ic ic-chev-down"></i><small>{more.label}</small>{/if}
      </button>
    {/if}
    {#if editing || !selected}
      <button class="tile arrange" class:on={editing}
        title={editing ? 'Finish arranging' : 'Drag the tools into your order'} onclick={() => (editing = !editing)}>
        <i class="ic {editing ? 'ic-check' : 'ic-grip'}"></i><small>{editing ? 'Done' : 'Order'}</small>
      </button>
    {/if}
  </div>
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
  .tool-row > .more { flex: 0 0 112px; background: var(--panel); }
  .editing .tiles .tile { cursor: grab; border-style: dashed; touch-action: none; }
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
