<script lang="ts">
  // The selection's place and size, each a button that asks for a new value.
  import { distance } from '../../lib/units.svelte';
  import type { Box } from '../../lib/svg';
  import type { Point } from '../../lib/transform';

  let { selection, zero, disabled, onplace, onturn, onresize }: {
    /** The selection's bounds, in drawing coordinates. */
    selection: Box;
    /** What the machine adds to a drawing coordinate. */
    zero: Point;
    disabled: boolean;
    onplace: (axis: 0 | 1) => void;
    onturn: () => void;
    onresize: (axis: 0 | 1 | 'scale') => void;
  } = $props();
</script>

<div class="canvas-hud">
  <button class="hud-btn" {disabled} data-numpad onclick={() => onplace(0)}>X {distance(selection.minX + zero[0])}</button>
  <button class="hud-btn" {disabled} data-numpad onclick={() => onplace(1)}>Y {distance(selection.minY + zero[1])}</button>
  <button class="hud-btn" {disabled} data-numpad onclick={onturn}>Rotate…</button>
  <button class="hud-btn" {disabled} onclick={() => onresize(0)}>W {distance(selection.maxX - selection.minX)}</button>
  <button class="hud-btn" {disabled} onclick={() => onresize(1)}>H {distance(selection.maxY - selection.minY)}</button>
  <button class="hud-btn" {disabled} onclick={() => onresize('scale')}>Scale…</button>
</div>
