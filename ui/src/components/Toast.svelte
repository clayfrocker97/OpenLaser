<script lang="ts">
  import { explain, technicalDetail } from '../lib/format';
  import { ui } from '../stores/ui.svelte';
  import { server } from '../stores/server.svelte';

  // The server's notes arrive with the document; each new one is shown once.
  let shown = $state(0);
  $effect(() => {
    const message = server.doc?.message;
    if (message && message.id !== shown) {
      shown = message.id;
      ui.say(explain(message.text), message.error);
    }
  });
</script>

{#if ui.toast}
  <div class="toast" class:error={ui.toast.error} role={ui.toast.error ? 'alert' : 'status'}>
    <span>{ui.toast.text}{#if technicalDetail(ui.toast.text)}<details class="toast-detail"><summary>Details</summary><code>{technicalDetail(ui.toast.text)}</code></details>{/if}</span>{#if ui.toast.error}<button class="toast-close" onclick={() => ui.dismissToast()}>OK</button>{/if}
  </div>
{/if}

<style>
  .toast-detail { margin-top: 4px; font-size: var(--t-sm); }
  .toast-detail summary { cursor: pointer; min-height: 44px; display: flex; align-items: center; }
  .toast-detail code { display: block; overflow-wrap: anywhere; opacity: .8; }
</style>
