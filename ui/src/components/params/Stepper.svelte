<script lang="ts">
  // A number with − and + either side; a tap on the number types a new one.
  // Values never go below zero.
  import { displayNumber, unitLabel } from '../../lib/units.svelte';
  import { osk } from '../../lib/osk.svelte';

  let { label, value, unit, step, disabled = false, apply }: {
    label: string;
    value: number;
    unit: string;
    step: number;
    disabled?: boolean;
    apply: (v: number) => void;
  } = $props();

  const down = () => apply(Math.max(0, +(value - step).toFixed(3)));
  const up = () => apply(+(value + step).toFixed(3));
  function type(): void {
    if (!disabled) osk.number(label, value, unit, (v) => apply(Math.max(0, v)));
  }
</script>

<div class="param"><div class="lbl">{label}</div>
  <div class="stepper">
    <button onclick={down} {disabled}>−</button>
    <div class="val tappable" data-numpad role="button" aria-disabled={disabled} tabindex={disabled ? -1 : 0}
      onclick={type} onkeydown={() => undefined}>{displayNumber(value, unit, step < 0.1 ? 2 : 1)}<small>{unitLabel(unit)}</small></div>
    <button onclick={up} {disabled}>+</button>
  </div>
</div>
