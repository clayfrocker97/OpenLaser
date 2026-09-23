<script lang="ts">
  // Gas & laser for the job being set up or run: the estimate for a number
  // of runs from the compiled program, and what past runs actually spent.
  // Every figure is priced by the server from Settings → Gas costs.
  import { api } from '../api/client';
  import type { RunRecord } from '../api';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { SETTINGS_PAGES } from '../lib/navigation';
  import { ago, explain } from '../lib/format';
  import { quantity } from '../lib/units.svelte';
  import { GAS_COLORS, GAS_NAMES, duration, gasLabel, litres, money, totals, volume } from '../lib/gas';

  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft);
  const gas = $derived(doc.gas);
  const currency = $derived(gas.costs.currency);
  const estimate = $derived(gas.estimate);
  let runs = $state(1);
  let history = $state<RunRecord[]>([]);
  let error = $state('');

  // Reload when a run is recorded or prices change (the gas revision), or
  // another job opens.
  // Primitives only: the document objects are replaced on every update.
  const draftKey = $derived(draft?.key ?? null);
  const draftJob = $derived(draft?.job ?? null);
  const gasRevision = $derived(gas.revision);
  $effect(() => {
    const key = draftKey, job = draftJob;
    void gasRevision;
    if (!key) { history = []; return; }
    let current = true;
    api.gasRuns(key, job).then(r => { if (current) { history = r; error = ''; } }, e => { if (current) error = explain(e); });
    return () => { current = false; };
  });

  const total = $derived(totals(history, currency));
  const perRun = $derived(estimate ? litres(estimate) : null);
  const step = (delta: number) => { runs = Math.min(99, Math.max(1, runs + delta)); };
  const settingsPage = SETTINGS_PAGES.find(p => p.groups.includes('gas'))?.id;
  function openSettings(): void {
    if (settingsPage === undefined) return;
    ui.machinePage = settingsPage;
    ui.tab = 'machine';
  }
  const unpriced = $derived(!!estimate && estimate.cost === null);
</script>

<div class="card2 gas-card">
  <div class="card2-head"><h3>Gas &amp; laser</h3>
    <div class="runs" role="group" aria-label="Runs">
      <button class="step" aria-label="One run fewer" disabled={runs <= 1} onclick={() => step(-1)}>−</button>
      <span class="count"><b>{runs}</b> {runs === 1 ? 'run' : 'runs'}</span>
      <button class="step" aria-label="One run more" disabled={runs >= 99} onclick={() => step(1)}>+</button>
    </div>
  </div>
  {#if estimate}
    <div class="stats">
      <div><b>{duration(estimate.laser * runs)}</b><span>Laser on</span></div>
      <div><b>{perRun === null ? '—' : volume(perRun * runs)}</b><span>{gasLabel(estimate)} used</span></div>
      <div><b>{estimate.cost === null ? '—' : money(estimate.cost * runs, currency)}</b><span>Gas cost</span></div>
    </div>
    <p class="detail">
      {#each estimate.gases as line, i}{#if i}, {/if}<span class="dot" style="background:{GAS_COLORS[line.gas]}"></span>{GAS_NAMES[line.gas]} {quantity(line.pressure, 'bar')}{/each}
      {#if estimate.nozzle} · {quantity(estimate.nozzle.diameter, 'mm', 1)} {estimate.nozzle.kind} nozzle{:else} · no nozzle in the recipe{/if}
      · {estimate.pierces * runs} pierces · {quantity(estimate.cut * runs / 1000, 'm', 1)} cut
    </p>
    {#if perRun === null || unpriced}
      <p class="note">{perRun === null ? 'Set the nozzle in the recipe, or a manual flow, to estimate gas.' : 'Enter gas prices to see costs.'}{#if settingsPage !== undefined} <button class="link-btn" onclick={openSettings}>Gas costs</button>{/if}</p>
    {/if}
  {:else}
    <p class="note">{draft?.recipe ? 'Estimates appear once the job is compiled.' : 'Choose a material to estimate gas and laser time.'}</p>
  {/if}

  <div class="past">
    <h4>Past runs</h4>
    {#if error}<p class="note warn-text">{error}</p>{/if}
    {#if history.length}
      <p class="summary">This job: {duration(total.laser)} laser, {volume(total.litres)} gas, {money(total.cost, total.currency)}</p>
      <div class="table" role="table" aria-label="Past runs">
        <div class="row head" role="row"><span role="columnheader">When</span><span role="columnheader"></span><span role="columnheader">Laser</span><span role="columnheader">Gas</span><span role="columnheader">Cost</span></div>
        {#each history as run (run.id)}
          <div class="row" role="row">
            <span role="cell">{ago(run.finished)}</span>
            <span role="cell"><span class="state" class:done={run.outcome === 'done'}>{run.outcome === 'done' ? 'Done' : `Stopped ${Math.round(run.fraction * 100)}%`}</span></span>
            <span role="cell">{duration(run.consumption.laser)}</span>
            <span role="cell">{volume(litres(run.consumption))}</span>
            <span role="cell">{money(run.consumption.cost, run.currency)}</span>
          </div>
        {/each}
      </div>
    {:else if !error}
      <p class="note">Costs appear here after the first run.</p>
    {/if}
  </div>
</div>

<style>
  .gas-card h4 { margin: 0; font-size: var(--t-sm); font-weight: 700; color: var(--ink-2); }
  .runs { display: flex; align-items: center; gap: 8px; }
  .step { width: 44px; height: 44px; border-radius: 10px; border: 1px solid var(--line); background: var(--panel-2); color: var(--ink); font-size: var(--t-lg); font-weight: 700; cursor: pointer; }
  .step:disabled { opacity: .4; cursor: default; }
  .count { min-width: 64px; text-align: center; font-size: var(--t-sm); color: var(--ink-3); }
  .count b { color: var(--ink); font-size: var(--t-base); }
  .gas-card .stats span { font-size: var(--t-sm); }
  .detail, .note, .summary { margin: 0; font-size: var(--t-sm); color: var(--ink-3); }
  .summary { color: var(--ink-2); font-weight: 600; }
  .dot { display: inline-block; width: 10px; height: 10px; border-radius: 50%; margin-right: 5px; vertical-align: -1px; }
  .link-btn { min-height: 44px; padding: 0 6px; border: 0; background: none; color: var(--accent-2); font-weight: 600; font-size: var(--t-sm); cursor: pointer; }
  .past { display: flex; flex-direction: column; gap: 8px; }
  .table { display: grid; gap: 2px; max-height: 240px; overflow-y: auto; overscroll-behavior: contain; }
  .row { display: grid; grid-template-columns: minmax(0, 1.3fr) auto repeat(3, minmax(0, 1fr)); gap: 8px; align-items: center; min-height: 36px; padding: 0 8px; border-radius: 8px; font-size: var(--t-sm); font-variant-numeric: tabular-nums; }
  .row:nth-child(even) { background: var(--panel-2); }
  .row.head { min-height: 28px; color: var(--ink-3); font-weight: 600; }
  .row > span { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row span:nth-child(n+3) { text-align: right; }
  .state { display: inline-block; padding: 2px 8px; border-radius: 999px; font-weight: 600; background: var(--hold-soft); color: var(--ink-2); white-space: nowrap; }
  .state.done { background: var(--move-soft); color: var(--move); }
</style>
