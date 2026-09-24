<script lang="ts">
  import type { Snippet } from 'svelte';

  // A dialog closes on its button or a completed tap on the backdrop, never
  // on a press that merely lands there; one that owns motion cannot close.
  let { title, wide = false, dismissable = true, onclose, actions, children }: { title: string; wide?: boolean; dismissable?: boolean; onclose: () => void; actions?: Snippet; children: Snippet } = $props();
  let pressed = false;
</script>

<div class="modal-backdrop" role="presentation"
  onpointerdown={(e) => { pressed = e.target === e.currentTarget; }}
  onpointerup={(e) => { if (pressed && dismissable && e.target === e.currentTarget) onclose(); pressed = false; }}>
  <div class="modal" class:wide role="dialog">
    <div class="modal-head"><h2>{title}</h2><div class="modal-head-actions">{#if actions}{@render actions()}{/if}<button
      class="btn btn-ghost icon-only" onclick={onclose} disabled={!dismissable} aria-label="Close"><i class="ic ic-x"></i></button></div></div>
    <div class="modal-body">{@render children()}</div>
  </div>
</div>

<style>
  .modal-head { gap: 12px; }
  .modal-head h2 { min-width: 0; }
  .modal-head-actions { display: flex; align-items: center; gap: 8px; flex: none; }
</style>
