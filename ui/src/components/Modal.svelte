<script lang="ts">
  import type { Snippet } from 'svelte';

  let { title, wide = false, onclose, actions, children }: { title: string; wide?: boolean; onclose: () => void; actions?: Snippet; children: Snippet } = $props();
</script>

<div class="modal-backdrop" role="presentation" onpointerdown={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="modal" class:wide role="dialog">
    <div class="modal-head"><h2>{title}</h2><div class="modal-head-actions">{#if actions}{@render actions()}{/if}<button class="btn btn-ghost icon-only" onclick={onclose} aria-label="Close"><i class="ic ic-x"></i></button></div></div>
    <div class="modal-body">{@render children()}</div>
  </div>
</div>

<style>
  .modal-head { gap: 12px; }
  .modal-head h2 { min-width: 0; }
  .modal-head-actions { display: flex; align-items: center; gap: 8px; flex: none; }
</style>
