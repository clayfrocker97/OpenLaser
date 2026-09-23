<script lang="ts">
  // Go to X/Y: type a position, then hold Go (DESIGN.md rule 1, held for
  // motion). Positions are in job coordinates when a job origin is set, or
  // machine coordinates; the server checks the target is on the bed.
  import Modal from './Modal.svelte';
  import HoldButton from './HoldButton.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain } from '../lib/format';
  import { plain } from '../lib/plain';
  import { distance, unitLabel } from '../lib/units.svelte';

  let { onclose }: { onclose: () => void } = $props();
  const doc = $derived(server.doc!);
  const origin = $derived(doc.draft?.origin ?? null);
  const position = $derived(doc.machine.feedback?.position_mm ?? null);
  let frame = $state<'job' | 'machine'>(server.doc?.draft?.origin ? 'job' : 'machine');
  const offset = $derived<[number, number]>(frame === 'job' && origin ? [origin[0], origin[1]] : [0, 0]);
  // Starts at the head's current position in the chosen frame.
  let target = $state<[number, number] | null>(null);
  const shown = $derived(target ?? (position ? [position[0] - offset[0], position[1] - offset[1]] as [number, number] : [0, 0] as [number, number]));
  const gate = $derived(doc.readiness.position);

  function edit(axis: 0 | 1): void {
    osk.number(`${frame === 'job' ? 'Job' : 'Machine'} ${axis === 0 ? 'X' : 'Y'}`, shown[axis], 'mm', (value) => {
      const next: [number, number] = [...shown];
      next[axis] = value;
      target = next;
    });
  }

  function chooseFrame(next: 'job' | 'machine'): void {
    if (next === frame) return;
    // Keep the same physical point when switching frames.
    const machine: [number, number] = [shown[0] + offset[0], shown[1] + offset[1]];
    frame = next;
    const shift: [number, number] = next === 'job' && origin ? [origin[0], origin[1]] : [0, 0];
    target = [machine[0] - shift[0], machine[1] - shift[1]];
  }

  async function go(): Promise<void> {
    try {
      await api.machine('go-xy', { xy: [shown[0] + offset[0], shown[1] + offset[1]] });
      onclose();
    } catch (error) { ui.say(explain(error), true); }
  }
</script>

<Modal title="Go to X/Y" {onclose}>
  <div class="goto">
    <div class="seg" role="group" aria-label="Coordinates">
      <button class:on={frame === 'job'} aria-pressed={frame === 'job'} disabled={!origin} onclick={() => chooseFrame('job')}>Job</button>
      <button class:on={frame === 'machine'} aria-pressed={frame === 'machine'} onclick={() => chooseFrame('machine')}>Machine</button>
    </div>
    <p class="muted">{frame === 'job' ? 'Measured from the job origin.' : origin ? 'Measured from the machine home.' : 'Measured from the machine home. Set a job origin to use job coordinates.'}</p>
    <div class="fields">
      {#each [0, 1] as axis (axis)}
        <button class="field" onclick={() => edit(axis as 0 | 1)}><span>{axis === 0 ? 'X' : 'Y'}</span><strong>{distance(shown[axis]!)}</strong><small>{unitLabel('mm')}</small></button>
      {/each}
    </div>
    {#if !gate.ok}<p class="gate-reason">{plain(gate.reason).text}</p>{/if}
    <HoldButton class="btn btn-move xl" disabled={!gate.ok || !server.link} onhold={go}>Go to X {distance(shown[0])} · Y {distance(shown[1])}</HoldButton>
  </div>
</Modal>

<style>
  .goto { display: grid; gap: 12px; min-width: min(420px, 100%); }
  .seg { display: flex; }
  .seg button { flex: 1; }
  .fields { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .field { display: grid; grid-template-columns: auto 1fr auto; align-items: baseline; gap: 8px; min-height: 64px; padding: 0 14px; border: 1px solid var(--line-2); border-radius: 12px; background: var(--panel); color: var(--ink); font: inherit; cursor: pointer; text-align: right; }
  .field span { color: var(--ink-3); font-weight: 700; text-align: left; }
  .field strong { font-size: var(--t-xl); font-variant-numeric: tabular-nums; }
  .field small { color: var(--ink-3); }
  p { margin: 0; font-size: var(--t-sm); }
</style>
