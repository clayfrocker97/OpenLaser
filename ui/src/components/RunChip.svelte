<script lang="ts">
  // The one status chip on the Run page, desktop and phone: the next setup
  // step, what the run is doing, or why Start is waiting. It never speaks for
  // alarms (the bell does): blocked only by alarms, it says Not ready.
  import StatusLine from './StatusLine.svelte';
  import { server } from '../stores/server.svelte';
  import { nextStep } from '../lib/next-step';
  import { isSetupAlarm } from '../lib/setup-alarms';
  import type { NamedGate } from '../lib/plain';

  let { message, gates = [], tone = 'info' }: {
    /** What the page would say without a setup step: the run's state or the Start gate. */
    message: string;
    /** Controls whose unavailability the details explain. */
    gates?: NamedGate[];
    tone?: 'info' | 'ready' | 'warn';
  } = $props();

  const doc = $derived(server.doc!);
  const program = $derived(doc.machine.program?.state ?? '');
  const active = $derived(['running', 'finishing', 'held'].includes(program));
  const step = $derived(nextStep(doc));
  const alarmReason = (reason: string | null | undefined) => !!reason && (/alarms? (are|is) active/.test(reason) || isSetupAlarm(reason));
  const text = $derived(step ?? (!active && alarmReason(doc.readiness.run.reason) ? 'Not ready' : message));
  const shownGates = $derived(active || step ? [] : gates.filter(([, gate]) => !alarmReason(gate.reason)));
</script>

<div class="run-chip"><StatusLine status={text} gates={shownGates} tone={step ? 'warn' : tone} alarms={false} /></div>

<style>
  .run-chip { min-width: 0; max-width: 420px; }
  .run-chip :global(.status-summary) { min-height: 52px; border: 0; border-radius: 12px; background: var(--panel); }
</style>
