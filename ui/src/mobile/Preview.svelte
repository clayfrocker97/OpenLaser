<script lang="ts">
  import { boxOf, viewBoxFor, pathOf, type Polyline } from '../lib/svg';
  let { outline, label = 'Job preview', small = false }: { outline: Polyline[]; label?: string; small?: boolean } = $props();
  const box = $derived(boxOf(outline));
  const viewbox = $derived(box ? viewBoxFor(box, small ? 1 : 1.7, 1, .12) : '0 0 100 60');
  const drawing = $derived(outline.map(line => pathOf(line)).join(' '));
</script>
<div class="preview" class:small>
  <svg viewBox={viewbox} role="img" aria-label={label}><path d={drawing} /></svg>
</div>
<style>
  .preview { width:100%; height:190px; padding:18px; background:linear-gradient(var(--grid) 1px,transparent 1px),linear-gradient(90deg,var(--grid) 1px,transparent 1px),var(--panel-2); background-size:22px 22px; overflow:hidden; border-radius:14px; }svg { width:100%; height:100%; display:block; }path { fill:none; stroke:var(--move); stroke-width:1.7px; stroke-linecap:round; stroke-linejoin:round; vector-effect:non-scaling-stroke; }.small { width:64px; height:64px; padding:7px; background:var(--panel-2); border:1px solid var(--line); border-radius:12px; }.small path { stroke:var(--ink-2); stroke-width:1.4px; }
</style>
