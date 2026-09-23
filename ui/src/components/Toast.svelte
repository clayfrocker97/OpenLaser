<script lang="ts">
  import { diagnosticText } from '../lib/units.svelte';
  import { ui } from '../stores/ui.svelte';
  import { server } from '../stores/server.svelte';

  // The server's notes arrive with the document; each new one is shown once.
  let shown = $state(0);
  $effect(() => {
    const message = server.doc?.message;
    if (message && message.id !== shown) {
      shown = message.id;
      ui.say(diagnosticText(message.text), message.error);
    }
  });
</script>

{#if ui.toast}
  <div class="toast" class:error={ui.toast.error} role={ui.toast.error ? 'alert' : 'status'}>
    <span>{ui.toast.text}</span>{#if ui.toast.error}<button class="toast-close" onclick={() => ui.dismissToast()}>OK</button>{/if}
  </div>
{/if}
