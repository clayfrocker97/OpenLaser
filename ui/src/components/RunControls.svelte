<script lang="ts">
  import { access } from '../lib/access.svelte';
  import { server } from '../stores/server.svelte';

  let { onaction, busy = false, showStop = true }: {
    onaction: (action:'run' | 'resume' | 'hold' | 'stop') => void;
    busy?: boolean;
    showStop?: boolean;
  } = $props();
  const doc = $derived(server.doc!);
  const intent = $derived(doc.can_resume || doc.machine.program?.state === 'held' || doc.recovery?.state === 'held' ? 'resume' : 'run');
  const ready = $derived(doc.readiness[intent]);
</script>

<div class="run-controls">
  <button class="btn btn-start xl" onclick={() => onaction(intent)} disabled={busy || !server.link || !access.canControl || !ready.ok} title={ready.reason ?? ''}><i class="ic ic-play"></i>{intent === 'resume' ? 'Resume' : 'Start'}</button>
  <button class="btn btn-hold xl" onclick={() => onaction('hold')} disabled={busy || !server.link || !doc.readiness.hold.ok} title={doc.readiness.hold.reason ?? ''}><i class="ic ic-pause"></i>Pause</button>
  {#if showStop}<button class="btn btn-stop xl" onclick={() => onaction('stop')} disabled={!server.link || !doc.readiness.stop.ok}><i class="ic ic-stop"></i>Stop</button>{/if}
</div>
