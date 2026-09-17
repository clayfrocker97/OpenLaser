<script lang="ts">
  import { boxOf, pathOf, viewBoxFor } from '../lib/svg';
  let { outline, cutouts = [], label = 'Sheet boundary and existing cutouts' }: { outline: number[][]; cutouts?: number[][][]; label?: string } = $props();
  const bounds = $derived(boxOf([outline]));
</script>
<svg aria-label={label} role="img" viewBox={bounds ? viewBoxFor(bounds, 4, 3, .08) : '0 0 400 300'}>
  <path d={pathOf(outline) + 'Z'} fill="var(--accent-soft)" stroke="var(--ink-3)" stroke-width="1.5" vector-effect="non-scaling-stroke" />
  {#each cutouts as points}<path d={pathOf(points) + 'Z'} fill="var(--panel-2)" stroke="var(--ink-3)" stroke-width="1" vector-effect="non-scaling-stroke" />{/each}
</svg>
<style>svg { width:100%; height:100%; min-height:0; display:block; }</style>
