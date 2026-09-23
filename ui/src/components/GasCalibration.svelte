<script lang="ts">
  // Calibrate a gas: note the supply, open the valve for 60 s at a chosen
  // pressure, measure what it used, and save measured / estimated as the
  // gas's calibration factor. Opening the valve is held (DESIGN.md rule 1);
  // Stop is one tap, and the dialog cannot close while gas flows (rule 12).
  import { api } from '../api/client';
  import type { GasKind, NozzleType } from '../api';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain } from '../lib/format';
  import { quantity } from '../lib/units.svelte';
  import { GAS_COLORS, GAS_NAMES, METHODS, measuredLitres, routesFor, volume, type Method } from '../lib/gas';
  import Modal from './Modal.svelte';
  import HoldButton from './HoldButton.svelte';

  let { gas, onclose }: { gas: GasKind; onclose: () => void } = $props();
  const TEST_SECONDS = 60;

  const doc = $derived(server.doc!);
  const routes = $derived(routesFor(doc.files.capabilities.find(c => c.laser === doc.mode)?.gases ?? [], gas));
  let selector = $state<number | null>(null);
  $effect(() => { if (routes.length && !routes.some(r => r.selector === selector)) selector = routes[0]!.selector; });
  const route = $derived(routes.find(r => r.selector === selector) ?? null);
  const bound = $derived(selector !== null && !!doc.bindings?.outputs.gas[selector]);

  // Start from the job's nozzle and this gas's pressure in it, when there is one.
  // Read once: the test starts from the job as it was when opened.
  const recipe = server.doc?.draft?.recipe?.attributes ?? {};
  const line = server.doc?.gas.estimate?.gases.find(g => g.gas === gas);
  let pressure = $state(line?.pressure ?? 1);
  let diameter = $state(Number(recipe['OpenLaserNozzleDiameter']) > 0 ? Number(recipe['OpenLaserNozzleDiameter']) : 1.5);
  let nozzleType = $state<NozzleType>(recipe['OpenLaserNozzleType'] === 'double' ? 'double' : 'single');
  let method = $state<Method>('pressure');
  let before = $state(0);
  let after = $state(0);
  let capacity = $state(50);

  let step = $state<'setup' | 'running' | 'measure' | 'saved'>('setup');
  let started = 0;
  let seen = false;
  let elapsed = $state(TEST_SECONDS);
  let expected = $state<number | null>(null);
  let factor = $state<number | null>(null);
  let busy = $state(false);

  const test = $derived({ gas, pressure, nozzle: { diameter, kind: nozzleType }, seconds: Math.min(TEST_SECONDS, Math.max(1, elapsed)) });
  $effect(() => {
    const query = $state.snapshot(test);
    let current = true;
    api.gasFlow(query).then(r => { if (current) expected = r.estimated; }, () => { if (current) expected = null; });
    return () => { current = false; };
  });

  // The valve plan runs as the controller's output operation.
  const operation = $derived(doc.machine.operation);
  const running = $derived(step === 'running');
  $effect(() => {
    if (step !== 'running') return;
    if (operation?.kind === 'outputs') { seen = true; return; }
    if (seen || Date.now() - started > 5000) {
      elapsed = Math.min(TEST_SECONDS, Math.round((Date.now() - started) / 1000));
      step = 'measure';
    }
  });
  const remaining = $derived(running ? Math.max(0, TEST_SECONDS - Math.floor(operation?.age_seconds ?? 0)) : TEST_SECONDS);

  async function open(): Promise<void> {
    if (selector === null) return;
    try {
      started = Date.now(); seen = false; elapsed = TEST_SECONDS;
      step = 'running';
      await api.machine('gas-calibration', { gas_calibration: { selector, pressure: route?.pressure ? pressure : 0 } });
    } catch (e) { step = 'setup'; ui.say(explain(e), true); }
  }
  async function stop(): Promise<void> {
    try { await api.machine('stop'); } catch (e) { ui.say(explain(e), true); }
  }
  const needsBefore = $derived((method === 'pressure' || method === 'weight') && before <= 0);
  const readings = $derived({ before, after, capacity, seconds: elapsed });
  const measured = $derived(measuredLitres(method, gas, readings));
  async function save(): Promise<void> {
    if (measured === null) return;
    busy = true;
    try {
      const saved = await api.calibrateGas({ ...$state.snapshot(test), measured });
      factor = saved.factor;
      step = 'saved';
    } catch (e) { ui.say(explain(e), true); }
    finally { busy = false; }
  }
  const num = (label: string, value: number, unit: string, set: (v: number) => void, min = 0) =>
    osk.number(label, value, unit, v => { if (Number.isFinite(v) && v >= min) set(v); });
  const beforeLabel = $derived(method === 'pressure' ? 'Cylinder pressure before' : method === 'weight' ? 'Cylinder weight before' : method === 'meter' ? 'Flow meter reading' : 'Litres used');
  const unit = $derived(method === 'pressure' ? 'bar' : method === 'weight' ? 'kg' : method === 'meter' ? 'L/min' : 'L');
</script>

<Modal title="Calibrate {GAS_NAMES[gas]}" dismissable={!running} {onclose}>
  <div class="cal" style="--gas:{GAS_COLORS[gas]}">
    <ol class="steps">
      <li class:on={step === 'setup'}>Note the supply</li>
      <li class:on={step === 'running'}>Open 60 s</li>
      <li class:on={step === 'measure' || step === 'saved'}>Measure</li>
    </ol>

    {#if step === 'setup'}
      <p class="muted">Fit the nozzle, keep the head clear of the sheet and nothing else on this gas. Then note the supply before opening the valve.</p>
      {#if routes.length > 1}
        <div class="choices" role="group" aria-label="Valve">{#each routes as r (r.selector)}<button class="btn" class:btn-primary={r.selector === selector} aria-pressed={r.selector === selector} onclick={() => (selector = r.selector)}>{r.name}</button>{/each}</div>
      {/if}
      <div class="fields">
        <button class="field" onclick={() => num('Test pressure', pressure, 'bar', v => { if (v > 0 && v <= 100) pressure = v; })}><span>{route?.pressure ? 'Pressure' : 'Regulator pressure'}</span><strong>{quantity(pressure, 'bar')}</strong></button>
        <button class="field" onclick={() => num('Nozzle diameter', diameter, 'mm', v => { if (v > 0 && v <= 20) diameter = v; })}><span>Nozzle</span><strong>{quantity(diameter, 'mm', 1)}</strong></button>
        <div class="field seg-field"><span>Nozzle type</span><div class="seg"><button class:on={nozzleType === 'single'} onclick={() => (nozzleType = 'single')}>Single</button><button class:on={nozzleType === 'double'} onclick={() => (nozzleType = 'double')}>Double</button></div></div>
      </div>
      <div class="seg methods" role="group" aria-label="How you measure">{#each METHODS as m (m.id)}<button class:on={method === m.id} aria-pressed={method === m.id} onclick={() => (method = m.id)}>{m.label}</button>{/each}</div>
      <p class="muted">{METHODS.find(m => m.id === method)?.hint}{method === 'pressure' ? '. Let the gauge settle for a few minutes after the test.' : '.'}</p>
      {#if method === 'pressure' || method === 'weight'}
        <div class="fields">
          <button class="field" onclick={() => num(beforeLabel, before, unit, v => (before = v))}><span>{beforeLabel}</span><strong>{quantity(before, unit, 1)}</strong></button>
          {#if method === 'pressure'}<button class="field" onclick={() => num('Cylinder water capacity', capacity, 'L', v => { if (v > 0) capacity = v; })}><span>Water capacity</span><strong>{quantity(capacity, 'L', 0)}</strong></button>{/if}
        </div>
      {/if}
      <p class="expect">The estimate for 60 s is <b>{expected === null ? '—' : volume(expected)}</b>.</p>
      {#if !route}<p class="gate-reason">This machine has no {GAS_NAMES[gas]} valve.</p>
      {:else if !bound}<p class="gate-reason">The {route.name} valve has no output assigned.</p>
      {:else if !doc.readiness.outputs.ok}<p class="gate-reason">{doc.readiness.outputs.reason ?? 'The machine is not ready for outputs.'}</p>
      {:else if needsBefore}<p class="gate-reason">Enter the {method === 'pressure' ? 'cylinder pressure' : 'cylinder weight'} before the test first.</p>{/if}
      <HoldButton class="btn btn-warn lg block" disabled={!route || !bound || !doc.readiness.outputs.ok || needsBefore} onhold={open}>Open gas for 60 s</HoldButton>
    {:else if step === 'running'}
      <div class="running">
        <div class="countdown"><b>{remaining}</b><span>s left</span></div>
        <p>{GAS_NAMES[gas]} is flowing at {quantity(pressure, 'bar')}.{#if method === 'meter'} Read the flow meter now.{/if}</p>
        <button class="btn btn-stop xl block" onclick={stop}>Stop</button>
      </div>
    {:else if step === 'measure'}
      {#if elapsed < TEST_SECONDS - 2}<p class="warn-text">The valve closed after {elapsed} s; the estimate below covers {elapsed} s.</p>{/if}
      <div class="fields">
        {#if method === 'pressure' || method === 'weight'}
          <div class="field static"><span>{beforeLabel}</span><strong>{quantity(before, unit, 1)}</strong></div>
          <button class="field" onclick={() => num(method === 'pressure' ? 'Cylinder pressure after' : 'Cylinder weight after', after, unit, v => (after = v))}><span>After</span><strong>{quantity(after, unit, 1)}</strong></button>
        {:else}
          <button class="field" onclick={() => num(beforeLabel, before, unit, v => (before = v))}><span>{beforeLabel}</span><strong>{quantity(before, unit, 1)}</strong></button>
        {/if}
      </div>
      <div class="stats">
        <div><b>{volume(measured)}</b><span>Measured</span></div>
        <div><b>{expected === null ? '—' : volume(expected)}</b><span>Estimated</span></div>
        <div><b>{measured !== null && expected ? `×${(measured / expected).toFixed(2)}` : '—'}</b><span>Factor</span></div>
      </div>
      <div class="row-actions">
        <button class="btn btn-ghost" onclick={() => (step = 'setup')}>Test again</button>
        <button class="btn btn-primary" disabled={measured === null || busy} onclick={save}>Save calibration</button>
      </div>
    {:else}
      <p class="done">{GAS_NAMES[gas]} estimates now use ×{factor?.toFixed(2)}.</p>
      <button class="btn btn-primary block" onclick={onclose}>Done</button>
    {/if}
  </div>
</Modal>

<style>
  .cal { display: flex; flex-direction: column; gap: 12px; min-width: min(520px, 90vw); }
  .steps { display: flex; gap: 8px; margin: 0; padding: 0; list-style: none; counter-reset: step; }
  .steps li { flex: 1; padding: 8px 10px; border-radius: 10px; background: var(--panel-2); color: var(--ink-3); font-size: var(--t-sm); font-weight: 600; counter-increment: step; }
  .steps li::before { content: counter(step) ". "; }
  .steps li.on { background: var(--panel); color: var(--ink); box-shadow: inset 0 -3px 0 var(--gas); }
  .muted, .expect, .done { margin: 0; font-size: var(--t-sm); }
  .done { font-size: var(--t-base); font-weight: 600; }
  .choices { display: flex; flex-wrap: wrap; gap: 8px; }
  .fields { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; }
  .field { display: grid; gap: 6px; min-height: 64px; padding: 12px 14px; text-align: left; border: 1px solid var(--line); border-radius: 12px; background: var(--panel-2); color: var(--ink); cursor: pointer; }
  .field.static { cursor: default; }
  .field > span { color: var(--ink-3); font-size: var(--t-sm); }
  .field strong { font-size: var(--t-base); white-space: nowrap; }
  .seg-field .seg button { min-height: 44px; padding: 0 12px; }
  .methods { display: flex; flex-wrap: wrap; }
  .methods button { min-height: 44px; flex: 1; }
  .running { display: flex; flex-direction: column; align-items: center; gap: 12px; text-align: center; }
  .countdown { display: flex; align-items: baseline; gap: 6px; }
  .countdown b { font-size: var(--t-xl); font-variant-numeric: tabular-nums; color: var(--gas); }
  .countdown span { font-size: var(--t-base); color: var(--ink-3); }
  .running p { margin: 0; }
  .row-actions { display: flex; justify-content: flex-end; gap: 8px; flex-wrap: wrap; }
  .stats span { font-size: var(--t-sm); }
</style>
