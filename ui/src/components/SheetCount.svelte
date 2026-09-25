<script lang="ts">
  // A count of sheets: fewer, the number (tap to type it) and more.
  import { osk } from '../lib/osk.svelte';

  let { label, prompt, value, min = 0, disabled = false, onchange }: {
    label: string;
    /** The keypad's title. */
    prompt: string;
    value: number;
    min?: number;
    disabled?: boolean;
    onchange: (value: number) => void;
  } = $props();
  const set = (v: number) => onchange(Math.max(min, Math.round(v)));
</script>

<div class="count">
  <span>{label}</span>
  <button aria-label="One fewer" disabled={disabled || value <= min} onclick={() => set(value - 1)}><i class="ic ic-minus"></i></button>
  <button class="number" {disabled} onclick={() => osk.number(prompt, value, 'sheets', set)}>{value}</button>
  <button aria-label="One more" {disabled} onclick={() => set(value + 1)}><i class="ic ic-plus"></i></button>
</div>

<style>
  .count { display:flex; align-items:center; gap:8px; }
  .count span { flex:1; font-size:var(--t-base); }
  .count button { min-width:52px; min-height:52px; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); cursor:pointer; }
  .count .number { min-width:76px; font-size:var(--t-xl); }
</style>
