<script lang="ts">
  // The one confirmation dialog: Cancel beside a button that names the
  // consequence. It closes on Cancel, Escape or a completed tap on the
  // backdrop, and answers no if this screen loses control.
  import { access } from '../lib/access.svelte';
  import { ui } from '../stores/ui.svelte';

  let pressed = false;
  $effect(() => { if (ui.confirmation && !access.canControl) ui.answer(false); });
</script>

<svelte:window onkeydown={(event) => { if (ui.confirmation && event.key === 'Escape') ui.answer(false); }} />

{#if ui.confirmation}
  {@const asked = ui.confirmation}
  <div class="modal-backdrop confirm-backdrop" role="presentation"
    onpointerdown={(event) => { pressed = event.target === event.currentTarget; }}
    onpointerup={(event) => { if (pressed && event.target === event.currentTarget) ui.answer(false); pressed = false; }}>
    <div class="modal confirm" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title" aria-describedby="confirm-body">
      <div class="confirm-body">
        <h2 id="confirm-title">{asked.title}</h2>
        <p id="confirm-body">{asked.body}</p>
      </div>
      <div class="confirm-actions">
        <button class="btn btn-ghost lg" onclick={() => ui.answer(false)}>Cancel</button>
        <button class="btn lg {asked.danger ? 'btn-danger' : 'btn-primary'}" onclick={() => ui.answer(true)}>{asked.confirm}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .confirm-backdrop { z-index:150; }
  .confirm { width:min(460px, 92vw); }
  .confirm-body { display:grid; gap:10px; padding:22px 22px 8px; }
  .confirm-body h2 { font-size:var(--t-lg); }
  .confirm-body p { margin:0; color:var(--ink-2); line-height:1.5; overflow-wrap:anywhere; }
  .confirm-actions { display:grid; grid-template-columns:1fr 1fr; gap:12px; padding:18px 22px 22px; }
  .confirm-actions .btn { min-width:0; white-space:normal; }
</style>
