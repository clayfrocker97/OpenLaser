<script lang="ts">
  import { orderPosition, orderSegments } from '../../lib/cut-order';
  import type { Preview } from '../../api';

  let { preview, disabled = false, progress = $bindable(0) }: { preview: Preview | null; disabled?: boolean; progress?: number } = $props();
  let playing = $state(false);
  const segments = $derived(orderSegments(preview));
  const position = $derived(orderPosition(segments, progress));
  $effect(() => { void preview; playing = false; progress = 0; });
  $effect(() => { if (disabled) playing = false; });
  $effect(() => {
    if (!playing) return;
    let last = performance.now(), frame = 0;
    const tick = (now: number) => {
      progress = Math.min(1, progress + (now - last) / 20_000);
      last = now;
      if (progress >= 1) playing = false;
      else frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  });
</script>

<div class="order-preview">
  <div class="row"><strong>Order preview</strong><span class="muted">Path {position ? position.contour + 1 : 0} / {preview?.contours.length ?? 0}</span></div>
  <input aria-label="Cut order preview position" type="range" min="0" max="1" step="0.001" bind:value={progress} disabled={disabled || !segments.length} oninput={() => (playing = false)} />
  <button class="btn btn-soft" disabled={disabled || !segments.length} onclick={() => { if (progress >= 1) progress = 0; playing = !playing; }}>{playing ? 'Pause preview' : 'Play cut order'}</button>
</div>

<style>
  .order-preview { display:grid; gap:12px; padding-top:16px; border-top:1px solid var(--line); }
  .row { justify-content:space-between; font-size:13px; }
  input { width: 100%; height: 32px; margin: 0; accent-color: var(--accent); cursor: pointer; }
</style>
