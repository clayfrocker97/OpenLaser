<script lang="ts">
  // Settings → Gas costs: where each gas comes from, its price, how its
  // flow is known and its calibration. Each change saves at once; the server
  // keeps them in the data directory.
  import { api } from '../api/client';
  import type { GasCosts, GasKind, Source } from '../api';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { GASES, GAS_COLORS, GAS_NAMES, calibrationStatus, money, pricePerM3 } from '../lib/gas';
  import GasCalibration from './GasCalibration.svelte';
  import { withBusy } from '../lib/busy';
  import { quantity, toDisplay, unitLabel } from '../lib/units.svelte';

  const saved = $derived(server.doc!.gas.costs);
  const problem = $derived(server.doc!.gas.error);
  let busy = $state(false);
  let calibrating = $state<GasKind | null>(null);
  const costs = $derived(saved);

  // Each change saves at once against the values it was made from.
  async function change(apply: (copy: GasCosts) => void): Promise<void> {
    const base = $state.snapshot(saved);
    const next = $state.snapshot(saved);
    apply(next);
    if (JSON.stringify(next) === JSON.stringify(base)) return;
    await withBusy((b) => (busy = b), () => api.saveGasCosts(next, base));
  }
  function number(label: string, value: number, unit: string, commit: (v: number) => void, min = 0): void {
    osk.number(label, value, unit, v => {
      if (Number.isFinite(v) && v >= min) commit(v);
      else ui.say(`Enter ${min} or more.`, true);
    });
  }
  function setSource(gas: GasKind, kind: Source['kind']): void {
    change(c => {
      const current = c[gas].source;
      if (current.kind === kind) return;
      c[gas].source = kind === 'refill' ? { kind, price: 0, volume: 10 }
        : kind === 'bulk' ? { kind, price_per_m3: pricePerM3(current) ?? 0 }
        : { kind, cost_per_hour: 0 };
    });
  }
  const priceText = (gas: GasKind): string => {
    const source = costs[gas].source;
    if (source.kind === 'compressor') {
      return source.cost_per_hour > 0 ? `${money(source.cost_per_hour, costs.currency)} per hour open` : 'No running cost yet';
    }
    const m3 = pricePerM3(source);
    return m3 === null ? 'No price yet' : `${money(toDisplay(m3, '/m³'), costs.currency)} per ${unitLabel('m³')}`;
  };
  type Refill = Extract<Source, { kind: 'refill' }>;
  type Bulk = Extract<Source, { kind: 'bulk' }>;
  type Compressor = Extract<Source, { kind: 'compressor' }>;
  function editCurrency(): void {
    osk.text('Currency symbol', costs.currency, v => {
      const t = v.trim();
      if (t && [...t].length <= 4) change(c => { c.currency = t; });
      else ui.say('Use 1 to 4 characters.', true);
    });
  }
  function editRefillPrice(gas: GasKind, source: Refill): void {
    number(`${GAS_NAMES[gas]} price per refill`, source.price, costs.currency,
      v => change(c => { c[gas].source = { ...source, price: v }; }));
  }
  function editVolume(gas: GasKind, source: Refill): void {
    number(`${GAS_NAMES[gas]} cylinder volume`, source.volume, 'm³',
      v => { if (v > 0) change(c => { c[gas].source = { ...source, volume: v }; }); });
  }
  function editBulkPrice(gas: GasKind, source: Bulk): void {
    number(`${GAS_NAMES[gas]} price per ${unitLabel('m³')}`, source.price_per_m3, '/m³',
      v => change(c => { c[gas].source = { ...source, price_per_m3: v }; }));
  }
  function editRunningCost(gas: GasKind, source: Compressor): void {
    number('Compressor running cost per hour', source.cost_per_hour, `${costs.currency}/h`,
      v => change(c => { c[gas].source = { ...source, cost_per_hour: v }; }));
  }
  function estimateFlow(gas: GasKind): void {
    change(c => { c[gas].flow = { kind: 'estimated' }; });
  }
  function manualFlow(gas: GasKind): void {
    change(c => { if (c[gas].flow.kind !== 'manual') c[gas].flow = { kind: 'manual', rate: 100 }; });
  }
  function editFlowRate(gas: GasKind, rate: number): void {
    number(`${GAS_NAMES[gas]} flow`, rate, 'L/min',
      v => { if (v > 0) change(c => { c[gas].flow = { kind: 'manual', rate: v }; }); });
  }
</script>

<div class="setting-group gas-costs"><h3>Gas costs</h3>
  <p class="intro">
    Prices turn each compiled job's gas time into money on the Setup screen.
    Flow is estimated from the recipe's nozzle and pressure unless you enter one; calibrate it with a 60-second test.
  </p>
  {#if problem}<p class="warn-text intro">{problem}</p>{/if}
  <div class="setting">
    <div class="lbl">Currency symbol<small>Shown before every amount</small></div>
    <button class="val" onclick={editCurrency}>{costs.currency}</button>
  </div>

  <div class="supplies">
  {#each GASES as gas (gas)}
    {@const supply = costs[gas]}
    <section class="supply" style="--gas:{GAS_COLORS[gas]}">
      <div class="gas-head"><span class="swatch"></span><strong>{GAS_NAMES[gas]}</strong><span class="derived">{priceText(gas)}</span></div>
      <div class="setting"><div class="lbl">Source</div>
        <div class="seg" role="group" aria-label="{GAS_NAMES[gas]} source">
          {#if gas === 'air'}
            <button
              class:on={supply.source.kind === 'compressor'}
              aria-pressed={supply.source.kind === 'compressor'}
              onclick={() => setSource(gas, 'compressor')}
            >Compressor</button>
          {/if}
          <button class:on={supply.source.kind === 'refill'} aria-pressed={supply.source.kind === 'refill'} onclick={() => setSource(gas, 'refill')}>
            {gas === 'air' ? 'Cylinder' : 'Refills'}
          </button>
          {#if gas !== 'air'}
            <button class:on={supply.source.kind === 'bulk'} aria-pressed={supply.source.kind === 'bulk'} onclick={() => setSource(gas, 'bulk')}>
              Bulk per {unitLabel('m³')}
            </button>
          {/if}
        </div>
      </div>
      {#if supply.source.kind === 'refill'}
        {@const source = supply.source}
        <div class="setting">
          <div class="lbl">Price per refill</div>
          <button class="val" data-numpad onclick={() => editRefillPrice(gas, source)}>{money(source.price, costs.currency)}</button>
        </div>
        <div class="setting">
          <div class="lbl">Cylinder volume<small>Gas content at standard conditions</small></div>
          <button class="val" data-numpad onclick={() => editVolume(gas, source)}>{quantity(source.volume, 'm³', 1)}</button>
        </div>
      {:else if supply.source.kind === 'bulk'}
        {@const source = supply.source}
        <div class="setting">
          <div class="lbl">Price per {unitLabel('m³')}</div>
          <button class="val" data-numpad onclick={() => editBulkPrice(gas, source)}>{money(toDisplay(source.price_per_m3, '/m³'), costs.currency)}</button>
        </div>
      {:else}
        {@const source = supply.source}
        <div class="setting">
          <div class="lbl">Running cost<small>Charged per hour the valve is open</small></div>
          <button class="val" data-numpad onclick={() => editRunningCost(gas, source)}>{money(source.cost_per_hour, costs.currency)}/h</button>
        </div>
      {/if}
      <div class="setting"><div class="lbl">Flow<small>{unitLabel('L/min')}</small></div>
        <div class="seg" role="group" aria-label="{GAS_NAMES[gas]} flow">
          <button class:on={supply.flow.kind === 'estimated'} aria-pressed={supply.flow.kind === 'estimated'} onclick={() => estimateFlow(gas)}>
            Nozzle estimate
          </button>
          <button class:on={supply.flow.kind === 'manual'} aria-pressed={supply.flow.kind === 'manual'} onclick={() => manualFlow(gas)}>Manual</button>
        </div>
      </div>
      {#if supply.flow.kind === 'manual'}
        {@const rate = supply.flow.rate}
        <div class="setting">
          <div class="lbl">Flow rate<small>Used for every nozzle and pressure</small></div>
          <button class="val" data-numpad onclick={() => editFlowRate(gas, rate)}>{quantity(rate, 'L/min', 1)}</button>
        </div>
      {/if}
      <div class="setting">
        <div class="lbl">Calibration<small class:calibrated={!!supply.calibration}>{calibrationStatus(costs, gas)}</small></div>
        <button class="btn btn-ghost" disabled={busy} onclick={() => (calibrating = gas)}>Calibrate</button>
      </div>
    </section>
  {/each}
  </div>

</div>

{#if calibrating}<GasCalibration gas={calibrating} onclose={() => (calibrating = null)} />{/if}

<style>
  .intro { margin: 0; font-size: var(--t-sm); color: var(--ink-3); }
  .gas-costs { grid-column: 1 / -1; }
  .supplies { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 12px; }
  .supply { display: flex; flex-direction: column; gap: 6px; padding: 12px 14px 4px; border: 1px solid var(--line); border-radius: var(--r-s); }
  .gas-head { display: flex; align-items: center; gap: 10px; min-height: 32px; }
  .gas-head strong { font-size: var(--t-base); }
  .swatch { width: 14px; height: 14px; border-radius: 50%; background: var(--gas); flex: none; }
  .derived { margin-left: auto; font-size: var(--t-sm); font-weight: 600; color: var(--ink-2); }
  .supply .seg button { min-height: 44px; padding: 0 12px; }
  .supply .setting .lbl { min-width: 56px; }
  small.calibrated { color: var(--ink-2); }
</style>
