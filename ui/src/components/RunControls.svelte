<script lang="ts">
  // Start and Resume can set the machine moving at once, so they are held;
  // Pause and Stop are always a single tap.
  import { access } from '../lib/access.svelte';
  import { plain } from '../lib/plain';
  import { server } from '../stores/server.svelte';
  import HoldButton from './HoldButton.svelte';

  let { onaction, busy = false, showStop = true, explain = true }: {
    onaction: (action:'run' | 'resume' | 'hold' | 'stop') => void;
    busy?: boolean;
    showStop?: boolean;
    /** Show why Start is unavailable; off where the page already says. */
    explain?: boolean;
  } = $props();
  const doc = $derived(server.doc!);
  const intent = $derived(doc.can_resume || doc.machine.program?.state === 'held' || doc.recovery?.state === 'held' ? 'resume' : 'run');
  const ready = $derived(doc.readiness[intent]);
  const running = $derived(['running', 'finishing'].includes(doc.machine.program?.state ?? ''));
</script>

<div class="run-controls">
  <HoldButton class="btn btn-start xl" onhold={() => onaction(intent)} disabled={busy || !server.link || !access.canControl || !ready.ok} title={plain(ready.reason).text}><i class="ic ic-play"></i>{intent === 'resume' ? 'Resume' : 'Start'}</HoldButton>
  <button class="btn btn-hold xl" onclick={() => onaction('hold')} disabled={busy || !server.link || !doc.readiness.hold.ok} title={doc.readiness.hold.reason ?? ''}><i class="ic ic-pause"></i>Pause</button>
  {#if showStop}<button class="btn btn-stop xl" onclick={() => onaction('stop')} disabled={!server.link || !doc.readiness.stop.ok}><i class="ic ic-stop"></i>Stop</button>{/if}
</div>
{#if explain && !busy && !running && !ready.ok && ready.reason}<p class="gate-reason run-reason">{plain(ready.reason).text}</p>{/if}
