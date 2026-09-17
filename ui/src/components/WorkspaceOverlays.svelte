<script lang="ts">
  import type { PostflightReview, PreflightReview } from '../api';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain } from '../lib/format';
  import FlightChecklist from './FlightChecklist.svelte';
  import Toast from './Toast.svelte';
  import Osk from './Osk.svelte';
  import NativeKeyboard from './NativeKeyboard.svelte';
  import Alarms from './Alarms.svelte';
  import Pending from './Pending.svelte';

  let { nativeKeyboard = false }: { nativeKeyboard?: boolean } = $props();
  let completion = $state<PostflightReview | null>(null);
  let dismissed = $state('');
  const notice = $derived(server.doc?.postflight?.id);
  let pause = $state<PreflightReview | null>(null);
  let seenPause: number | null = null;
  const pausedExecution = $derived(server.doc?.machine.program?.state === 'held' && server.doc.recovery?.ready ? server.doc.execution?.id ?? null : null);
  $effect(() => {
    const id = pausedExecution;
    if (id === null) { pause = null; return; }
    if (id === seenPause) return;
    let current = true;
    void api.preflight('resume').then(review => {
      if (current) { seenPause = id; if (review.steps.length) pause = review; }
    }).catch(error => ui.say(explain(error), true));
    return () => { current = false; };
  });
  $effect(() => {
    const id = notice;
    if (!id || id === dismissed) { completion = null; return; }
    let current = true;
    void api.postflight().then(review => {
      if (current && review?.notice.id === id) completion = review;
    }).catch(error => ui.say(explain(error), true));
    return () => { current = false; };
  });
</script>

<Toast />
{#if nativeKeyboard}<NativeKeyboard />{:else}<Osk />{/if}
{#if pause}<FlightChecklist initial={pause} onclose={() => { pause = null; }} />{/if}
{#if completion && notice === completion.notice.id}
  {#key completion.notice.id}<FlightChecklist initial={completion} onclose={() => { dismissed = completion?.notice.id ?? ''; completion = null; }} />{/key}
{/if}
{#if ui.modal === 'alarms'}<Alarms onclose={() => ui.modal = null} />{/if}
{#if ui.modal === 'pending' && server.doc}<Pending onclose={() => { ui.modal = null; ui.pendingJobName = null; }} />{/if}
