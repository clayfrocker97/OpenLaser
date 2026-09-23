<script lang="ts">
  import { quantity } from '../lib/units.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain } from '../lib/format';
  import HoldButton from './HoldButton.svelte';

  const doc = $derived(server.doc!);
  let pulseDuration = $state(100);
  let pulsePower = $state(10);
  let gas = $state(0);
  const gases = $derived(doc.files.capabilities.find(c => c.laser === doc.mode)?.gases.filter(g => g.valve) ?? []);
  const route = $derived(gases.find(g => g.selector === gas));
  $effect(() => { if (gases.length && !route) gas = gases[0]!.selector; });
  let pressure = $state(1);
  let gasDuration = $state(500);
  async function run(action: () => Promise<unknown>): Promise<void> {
    try { await action(); } catch (error) { ui.say(explain(error), true); }
  }
</script>

<div class="setting-group manual-tests"><h3>Manual tests</h3>
  <div class="setting"><div class="lbl">Laser pulse<small>Laser fires at the current position.</small></div></div>
  <div class="test-fields">
    <button class="test-value" onclick={() => osk.number('Pulse duration', pulseDuration, 'ms', v => { if (v >= 10 && v <= 1000) pulseDuration = Math.round(v); })}><span>Duration</span><strong>{pulseDuration} ms</strong></button>
    <button class="test-value" onclick={() => osk.number('Pulse power', pulsePower, '%', v => { if (v >= 1 && v <= 100) pulsePower = Math.round(v); })}><span>Power</span><strong>{pulsePower}%</strong></button>
    <HoldButton class="btn btn-danger" disabled={!doc.readiness.jog.ok} onhold={() => run(() => api.machine('pulse', { pulse: { duration_ms: pulseDuration, power: pulsePower } }))}>Pulse laser</HoldButton>
  </div>
  <div class="setting gas-label"><div class="lbl">Gas test<small>Timed valve test.</small></div></div>
  <div class="gas-choices" role="group" aria-label="Gas test route">{#each gases as { name, selector }}<button class="btn" class:btn-primary={gas === selector} aria-pressed={gas === selector} disabled={!doc.bindings?.outputs.gas[selector]} onclick={() => (gas = selector)}>{name}</button>{/each}</div>
  <div class="test-fields gas-fields">
    {#if route?.pressure}<button class="test-value" onclick={() => osk.number('Test pressure', pressure, 'bar', v => { if (v > 0 && v <= 100) pressure = v; })}><span>Pressure</span><strong>{quantity(pressure, 'bar')}</strong></button>{:else}<div class="test-value"><span>Pressure</span><strong>At regulator</strong></div>{/if}
    <button class="test-value" onclick={() => osk.number('Gas test duration', gasDuration, 'ms', v => { if (v >= 50 && v <= 2000) gasDuration = Math.round(v); })}><span>Duration</span><strong>{gasDuration} ms</strong></button>
    <HoldButton class="btn btn-warn" disabled={!doc.readiness.outputs.ok || !doc.bindings?.outputs.gas[gas]} onhold={() => run(() => api.machine('gas-test', { gas_test: { selector: gas, pressure, duration_ms: gasDuration } }))}>Test gas</HoldButton>
  </div>
</div>

<style>
  .test-fields { display: grid; grid-template-columns: 1fr 1fr 1.2fr; gap: 10px; }
  .gas-fields { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .gas-fields > :global(.btn) { grid-column: 1 / -1; }
  .gas-choices { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px; margin-bottom: 12px; }
  .test-value { display: grid; gap: 8px; text-align: left; min-width: 0; padding: 14px; border: 1px solid var(--line); border-radius: 12px; background: var(--panel-2); color: var(--ink); cursor: pointer; }
  .test-value > span { color: var(--ink-3); font-size: var(--t-xs); }
  .test-value strong { font-size: var(--t-base); white-space: nowrap; }
  .gas-label { margin-top: 20px; }
</style>
