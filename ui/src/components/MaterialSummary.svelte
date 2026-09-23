<script lang="ts">
  // The one material summary (lib/summary.ts): speed, power, duty,
  // frequency, gas and pressure, nozzle, focus, lens, cut height and
  // pierce, in that order and under those names on every screen. `grid`
  // lays every value out as a tile, `—` where the recipe does not say;
  // `line` runs the values that are set together as one wrapping line for
  // lists and narrow headers. Nothing here is a control.
  import { summaryItems, type SummarySource } from '../lib/summary';

  let { source, variant = 'grid' }: { source: SummarySource; variant?: 'grid' | 'line' } = $props();
  const items = $derived(summaryItems(source));
</script>

{#if variant === 'line'}
  <span class="summary-line">{#each items.filter((i) => i.set) as item, n (item.key)}{#if n}<span class="sep" aria-hidden="true">·</span>{' '}{/if}<span class="item"><span class="k">{item.label}</span> {item.value}</span>{/each}</span>
{:else}
  <dl class="summary-grid">
    {#each items as item (item.key)}
      <div class:unset={!item.set}><dt>{item.label}</dt><dd>{item.value}</dd></div>
    {/each}
  </dl>
{/if}

<style>
  .summary-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(112px, 1fr)); gap: 6px; margin: 0; }
  .summary-grid div { display: flex; flex-direction: column; gap: 2px; min-width: 0; padding: 8px 10px; border-radius: 10px; background: var(--panel-2); border: 1px solid var(--line); }
  .summary-grid dt { font-size: var(--t-sm); color: var(--ink-3); }
  .summary-grid dd { margin: 0; font-size: var(--t-base); font-weight: 700; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
  .summary-grid .unset dd { color: var(--ink-3); font-weight: 600; }
  .summary-line { font-size: var(--t-sm); color: var(--ink-2); line-height: 1.5; overflow-wrap: anywhere; }
  .summary-line .item { white-space: nowrap; font-variant-numeric: tabular-nums; }
  .summary-line .k { color: var(--ink-3); }
  .summary-line .sep { color: var(--ink-3); margin-left: .4em; }
</style>
