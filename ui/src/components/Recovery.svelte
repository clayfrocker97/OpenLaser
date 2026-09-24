<script lang="ts">
  import { distance as length, quantity, unitLabel } from '../lib/units.svelte';
  import { api } from '../api/client';
  import HoldButton from './HoldButton.svelte';
  import { server } from '../stores/server.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain, fmt } from '../lib/format';
  import type { RecoveryChange } from '../api';
  let { onresume, reviewing = false }: { onresume: () => void; reviewing?: boolean } = $props();
  const doc = $derived(server.doc!);
  const recovery = $derived(doc.recovery!);
  const selected = $derived(recovery.selected);
  const step = $derived(selected ? recovery.steps[selected.pass] : null);
  const completed = $derived(recovery.steps.filter(s => s.status === 'completed').length);
  const skipped = $derived(recovery.steps.filter(s => s.status === 'skipped').length);
  let page = $state<'main' | 'adjust' | 'paths' | 'prepare'>('main');
  let sending = $state(false), clearance = $state(false), error = $state(''), search = $state('');
  let distance = $state(1);
  const busy = $derived(!!doc.machine.operation || sending || reviewing);
  const label = (kind: string): string => kind === 'pre_pierce' ? 'Pre-pierce' : kind === 'film' ? 'Film removal' : 'Cut';
  const available = $derived(recovery.steps.flatMap((s, index) => s.status === 'skipped' ? [] : [{ ...s, index }]));
  const previous = $derived(selected ? available.some(s => s.index < selected.pass) : false);
  const next = $derived(selected ? available.some(s => s.index > selected.pass) : false);
  const shown = $derived(available.filter(s =>
    !search.trim()
    || `${s.pass.ordinal + 1} ${label(s.pass.kind)} ${s.status}`.toLowerCase().includes(search.trim().toLowerCase())));
  const title = $derived(
    page === 'main' ? 'Restart job'
    : page === 'adjust' ? 'Adjust restart'
    : page === 'paths' ? 'Choose a path'
    : 'Prepare restart');
  const stepName = $derived(step ? `${label(step.pass.kind)} ${step.pass.ordinal + 1}` : 'Choose a path');
  const along = $derived(selected ? `${fmt(selected.fraction * 100, 1)}% along the path` : 'Tap the drawing or choose here');
  async function call(action: () => Promise<unknown>, after?: () => void): Promise<void> {
    if (busy) return;
    sending = true; error = '';
    try { await action(); after?.(); } catch (e) { error = explain(e); } finally { sending = false; }
  }
  const change = (value: RecoveryChange, after?: () => void) =>
    call(() => api.recoveryChange(value, recovery.revision), after);
  const toMain = (): void => { page = 'main'; };
  /** Applies a change and returns to the main page once it succeeds. */
  const changeThenMain = (value: RecoveryChange) => change(value, toMain);
  function choose(pass: number): void {
    void changeThenMain({ kind: 'select', pass, fraction: 0 });
  }
  function stepDistance(): void {
    osk.number('Restart step', distance, 'mm', value => { if (value > 0 && value <= 10000) distance = value; });
  }
  function confirmPosition(): Promise<void> {
    return call(() => api.prepareRecovery(recovery.revision, clearance), toMain);
  }
  function stop(): void {
    void api.machine('stop').catch(e => error = explain(e));
  }
  function fraction(): void {
    if (!selected) return;
    const pass = selected.pass;
    osk.number('Position along this path', selected.fraction * 100, '%', value => {
      if (value >= 0 && value <= 100) void change({ kind: 'select', pass, fraction: value / 100 });
    });
  }
  function primary(): void {
    if (recovery.ready) onresume();
    else { page = 'prepare'; clearance = false; }
  }
  const resetKey = $derived(`${recovery.id}/${recovery.state}`);
  $effect(() => { void resetKey; clearance = false; page = 'main'; });
</script>

<aside class="panel side recovery-side" aria-label="Job recovery">
  <div class="recovery-heading">
    {#if page !== 'main'}<button class="back" disabled={busy} onclick={toMain} aria-label="Back to restart">‹</button>{/if}
    <div>
      <h2>{title}</h2>
      <p>{completed} of {recovery.steps.length} paths complete{skipped ? ` · ${skipped} skipped` : ''}</p>
    </div>
  </div>
  <div class="recovery-content">
    {#if page === 'main'}
      <p class="help">Tap a path on the drawing to choose where to restart. Completed paths stay grey.</p>
      <div class="restart-card">
        <span class="eyebrow">Restart from</span>
        <div class="path-nav">
          <button class="arrow" aria-label="Previous restart path" disabled={busy || !previous} onclick={() => change({ kind: 'previous' })}>‹</button>
          <button class="path-choice" disabled={busy} onclick={() => page = 'paths'}>
            <strong>{stepName}</strong>
            <span>{along}</span>
          </button>
          <button class="arrow" aria-label="Next restart path" disabled={busy || !next} onclick={() => change({ kind: 'next' })}>›</button>
        </div>
        {#if recovery.position}
          <p class="position">X {length(recovery.position[0])} · Y {length(recovery.position[1])} {unitLabel('mm')}</p>
        {/if}
      </div>
      <button class="row-action" disabled={busy || !selected} onclick={() => page = 'adjust'}>
        <span>Adjust restart point<small>Move back, skip an outline or recall a point</small></span>
        <span>›</span>
      </button>
      <HoldButton
        class="btn btn-move block"
        disabled={busy || !recovery.ready || !selected || !doc.readiness.jog.ok}
        onhold={() => call(() => api.moveRestart(recovery.revision))}>Move head here · laser off</HoldButton>
    {:else if page === 'paths'}
      <input class="path-search" aria-label="Find restart path" type="search" bind:value={search} placeholder="Path number or status…" />
      <div class="paths" role="group" aria-label="Restart paths">
        {#each shown as item}
          <button class="path-row" aria-pressed={selected?.pass === item.index} disabled={busy} onclick={() => choose(item.index)}>
            <strong>{label(item.pass.kind)} {item.pass.ordinal + 1}</strong>
            <span>{item.status} · {quantity(item.length_mm, 'mm', 1)}</span>
          </button>
        {/each}
      </div>
    {:else if page === 'adjust'}
      <p class="help">Fine-tune the point highlighted on the drawing.</p>
      <div class="distance">
        <span>Step distance</span>
        <button class="value" disabled={busy} onclick={stepDistance}>{quantity(distance, 'mm')}</button>
      </div>
      <div class="step-pair">
        <button class="btn" disabled={busy || !selected} onclick={() => change({ kind: 'backward', distance })}>← Back</button>
        <button class="btn" disabled={busy || !selected} onclick={() => change({ kind: 'forward', distance })}>Forward →</button>
      </div>
      <button class="row-action" disabled={busy || !selected} onclick={fraction}>
        <span>Position along path</span>
        <strong>{selected ? `${fmt(selected.fraction * 100, 1)}%` : '—'}</strong>
      </button>
      <div class="group"><span class="eyebrow">Saved points</span>
        {#if recovery.last_pause}
          <button class="row-action" disabled={busy} onclick={() => changeThenMain({ kind: 'last_pause' })}>
            <span>Return to last pause</span>
            <span>›</span>
          </button>
        {/if}
        {#if recovery.good}
          <button class="row-action" disabled={busy} onclick={() => changeThenMain({ kind: 'last_good' })}>
            <span>Return to marked point</span>
            <span>›</span>
          </button>
        {/if}
        <button class="row-action" disabled={busy || !selected} onclick={() => changeThenMain({ kind: 'mark_good' })}>
          <span>Mark this point as good</span>
          <span>＋</span>
        </button>
      </div>
      <div class="group">
        <span class="eyebrow">What to cut</span>
        <button class="row-action" disabled={busy || !selected} onclick={() => changeThenMain({ kind: 'skip' })}>
          <span>Skip this outline</span>
          <span>›</span>
        </button>
        {#if skipped}
          <button class="row-action" disabled={busy} onclick={() => changeThenMain({ kind: 'include_all' })}>
            <span>Restore skipped outlines</span>
            <span>{skipped}</span>
          </button>
        {/if}
      </div>
    {:else}
      <p class="help">Establish the machine reference, then confirm that the sheet still matches the drawing.</p>
      {#if !doc.machine.session.homed}
        <HoldButton
          class="btn btn-move block"
          disabled={busy || !doc.readiness.home.ok}
          onhold={() => call(() => api.machine('home'))}>Home machine</HoldButton>
      {/if}
      <label class="clearance">
        <input type="checkbox" bind:checked={clearance} disabled={busy} />
        <span>The sheet is in the same position and the restart path is clear.</span>
      </label>
      <button
        class="btn btn-primary block"
        disabled={busy || !clearance || !selected || !doc.readiness.jog.ok}
        onclick={confirmPosition}>Confirm restart position</button>
    {/if}
    {#if recovery.problem || error}<p role="alert" class="recovery-error">{error || recovery.problem}</p>{/if}
  </div>
  <div class="recovery-footer">
    {#if page === 'main' && recovery.ready}
      <HoldButton
        class="btn btn-start xl block"
        disabled={busy || !selected || !doc.readiness.resume.ok}
        onhold={primary}>{reviewing ? 'Preparing…' : 'Resume from here'}</HoldButton>
    {:else if page === 'main'}
      <button class="btn btn-start xl block" disabled={busy || !selected} onclick={primary}>Prepare restart</button>
    {:else if page !== 'prepare'}
      <button class="btn btn-primary block" disabled={busy} onclick={toMain}>Done</button>
    {/if}
    <button class="btn btn-ghost block stop-recovery" disabled={sending || !doc.readiness.stop.ok} onclick={stop}>Stop motion</button>
  </div>
</aside>

<style>
  .recovery-side { padding:0; display:flex; flex-direction:column; overflow:hidden; }
  .recovery-heading { display:flex; align-items:center; gap:12px; padding:24px 22px 18px; border-bottom:1px solid var(--line); }
  h2 { margin:0; color:var(--ink); font-size:var(--t-xl); font-weight:650; letter-spacing:-.5px; text-transform:none; }
  .recovery-heading p { margin:6px 0 0; color:var(--ink-3); font-size:var(--t-sm); line-height:1.5; }
  .back,.arrow { cursor:pointer; border:1px solid var(--line); background:var(--panel-2); border-radius:12px; color:var(--ink); min-width:52px; min-height:56px; font-size:var(--t-xl); }
  .recovery-content { padding:22px; display:flex; flex-direction:column; gap:18px; overflow-y:auto; flex:1; min-height:0; }
  .help { color:var(--ink-3); font-size:var(--t-base); line-height:1.6; margin:0; }
  .restart-card { padding:16px 12px; border:1px solid var(--line); border-radius:14px; background:var(--panel-2); }
  .eyebrow { font-size:var(--t-sm); text-transform:uppercase; letter-spacing:.1em; color:var(--ink-3); padding:0 4px; }
  .path-nav { display:grid; grid-template-columns:48px 1fr 48px; align-items:center; gap:8px; margin-top:10px; }
  .path-nav .arrow { border:0; min-width:48px; background:transparent; }
  .path-choice { background:transparent; border:0; color:var(--ink); padding:10px 0; cursor:pointer; min-height:76px; display:grid; gap:6px; }
  .path-choice strong { font-size:var(--t-lg); font-weight:650; } .path-choice span { font-size:var(--t-sm); color:var(--ink-3); }
  .position { font-variant-numeric:tabular-nums; text-align:center; margin:12px 0 0; color:var(--ink-3); font-size:var(--t-sm); }
  .row-action {
    width:100%; min-height:60px; display:flex; align-items:center; justify-content:space-between; gap:14px; padding:13px 4px; background:transparent;
    color:var(--ink); border:0; border-bottom:1px solid var(--line); text-align:left; cursor:pointer; font:inherit; font-size:var(--t-base);
  }
  .row-action small { display:block; color:var(--ink-3); margin-top:6px; line-height:1.5; font-size:var(--t-sm); }
  .row-action > span:last-child { flex:none; } .group { display:grid; gap:4px; }
  .recovery-content :global(.btn),.recovery-footer :global(.btn) { min-height:56px; font-size:var(--t-base); }
  .recovery-footer { display:grid; gap:10px; padding:18px 22px; border-top:1px solid var(--line); background:var(--panel); }
  .recovery-footer :global(.btn-start) { min-height:66px; font-size:var(--t-lg); } .stop-recovery { color:var(--stop); }
  .path-search { width:100%; min-height:52px; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); font:inherit; padding:12px; }
  .paths { display:grid; gap:8px; } .path-row { display:grid; gap:7px; min-height:72px; text-align:left; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); padding:14px; cursor:pointer; font:inherit; }
  .path-row span { font-size:var(--t-sm); color:var(--ink-3); } .path-row[aria-pressed="true"] { border-color:var(--accent); background:var(--accent-soft); }
  .distance { display:flex; align-items:center; justify-content:space-between; gap:12px; }
  .value { min-height:54px; padding:12px 18px; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); cursor:pointer; font:inherit; }
  .step-pair { display:grid; grid-template-columns:1fr 1fr; gap:12px; }
  .clearance { display:flex; align-items:flex-start; gap:14px; padding:16px; border:1px solid var(--line); border-radius:12px; min-height:84px; cursor:pointer; font-size:var(--t-base); line-height:1.6; }
  .clearance input { width:24px; height:24px; flex:none; margin-top:2px; accent-color:var(--accent); }
  .recovery-error { padding:12px; color:var(--warn); background:var(--warn-soft); border-radius:10px; font-size:var(--t-sm); line-height:1.6; margin:0; }
  .recovery-side :global(button:disabled) { cursor:default; opacity:.38; }
</style>
