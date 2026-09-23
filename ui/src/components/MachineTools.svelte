<script lang="ts">
  // Machine tests: things that fire the beam, open a valve or change the
  // machine, kept beside the jog pad rather than among the settings. Every
  // one is held (DESIGN.md rules 1 and 5); Stop stays in the top bar.
  import { onMount } from 'svelte';
  import Modal from './Modal.svelte';
  import ManualTests from './ManualTests.svelte';
  import HoldButton from './HoldButton.svelte';
  import { Hold } from '../lib/hold';
  import { access } from '../lib/access.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain, laserLabel } from '../lib/format';
  import { plain } from '../lib/plain';
  import { quantity } from '../lib/units.svelte';
  import type { OutputRequest } from '../api';

  let { onclose }: { onclose: () => void } = $props();
  const doc = $derived(server.doc!);
  const bindings = $derived(doc.bindings);
  const readiness = $derived(doc.readiness);

  async function run(action: () => Promise<unknown>): Promise<void> {
    try { await action(); } catch (error) { ui.say(explain(error), true); }
  }

  const held = new Hold({
    heartbeat: (lease) => api.machine('heartbeat', { lease }),
    release: (lease) => api.machine('release', { lease }),
  }, (error) => ui.say(explain(error), true), () => server.link && access.canControl);
  onMount(() => held.mount());
  $effect(() => { if (!server.link || !access.canControl) held.cancel(); });
  let gasPressure = $state(5);
  function hold(event: PointerEvent, output: OutputRequest): void {
    held.press(event, (lease) => api.machine('outputs', { output, lease }));
  }
  const GAS = ['Low air', 'Low O₂', 'Low N₂', 'High air', 'High O₂', 'High N₂'];
</script>

<Modal title="Machine tests" wide onclose={() => { held.cancel(); onclose(); }}>
  <div class="machine-tools">
    <div class="setting-group"><h3>Laser mode</h3>
      <div class="setting"><div class="lbl">Operating mode<small>Hold to switch · the XY reference is kept</small></div>
        <div class="seg">{#each ['fiber', 'co2'] as mode}{#if doc.mode === mode}<button class="on" disabled>{laserLabel(mode as 'fiber' | 'co2')}</button>{:else}<HoldButton class="" disabled={!readiness.mode.ok} title={plain(readiness.mode.reason).text} onhold={() => run(() => api.machine('mode', { mode: mode as 'fiber' | 'co2' }))}>{laserLabel(mode as 'fiber' | 'co2')}</HoldButton>{/if}{/each}</div>
      </div>
      {#if !readiness.mode.ok && readiness.mode.reason}<p class="gate-reason">{plain(readiness.mode.reason).text}</p>{/if}
    </div>

    <ManualTests />

    <div class="setting-group"><h3>Manual outputs</h3>
      <div class="setting"><div class="lbl">Hold a button<small>Release to turn off.</small></div></div>
      <div class="setting"><div class="lbl">Pointer<small>{laserLabel(doc.mode)} · {bindings?.outputs.pointer_port ? `Output ${bindings.outputs.pointer_port}` : 'Unassigned'}</small></div><button class="btn btn-ghost deadman" disabled={!bindings?.outputs.pointer || !readiness.outputs.ok} onpointerdown={(event) => hold(event, { kind: 'pointer' })}>Hold</button></div>
      <div class="setting"><div class="lbl">Shutter<small>{laserLabel(doc.mode)} · {bindings?.outputs.shutter_port ? `Output ${bindings.outputs.shutter_port}` : 'Unassigned'}</small></div><button class="btn btn-warn deadman" disabled={!bindings?.outputs.shutter || !readiness.outputs.ok} onpointerdown={(event) => hold(event, { kind: 'shutter' })}>Hold</button></div>
      {#each GAS as name, selector}
        {#if bindings?.outputs.gas[selector]}
          <div class="setting"><div class="lbl">{name}<small>valve{selector < 3 ? ' and proportional pressure' : ''}</small></div><button class="btn btn-warn deadman" disabled={!readiness.outputs.ok} onpointerdown={(event) => hold(event, { kind: 'gas', selector, pressure: gasPressure })}>Hold</button></div>
        {/if}
      {/each}
      <div class="setting"><div class="lbl">Gas pressure<small>for the proportional valve</small></div><button class="val" data-numpad onclick={() => osk.number('Gas pressure', gasPressure, 'bar', (v) => { if (v >= 0 && v <= 100) gasPressure = v; })}>{quantity(gasPressure, 'bar')}</button></div>
      {#if !readiness.outputs.ok && readiness.outputs.reason}<p class="gate-reason">{plain(readiness.outputs.reason).text}</p>{/if}
    </div>
  </div>
</Modal>

<style>
  .machine-tools { display: grid; gap: 16px; min-width: min(640px, 100%); }
</style>
