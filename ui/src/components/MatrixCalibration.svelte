<script lang="ts">
  import { distance, quantity, unitLabel } from '../lib/units.svelte';
  import { api } from '../api/client';
  import type { CorrectionView, LaserMode, Measurement } from '../api';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain, fmt, laserLabel } from '../lib/format';

  let mode = $state<LaserMode>(server.doc?.mode ?? 'fiber');
  let data = $state<CorrectionView | null>(null);
  let page = $state(0);
  let selected = $state(0);
  let x = $state<number | null>(null);
  let y = $state<number | null>(null);
  let busy = $state(false);
  let error = $state('');
  let saved = $state('');
  const order = [6, 7, 8, 3, 4, 5, 0, 1, 2];
  const count = $derived(data?.measurements.filter(Boolean).length ?? 0);
  const steps = ['Cut squares', 'Measure', 'Apply'];

  function choose(index: number): void {
    selected = index;
    x = data?.measurements[index]?.x ?? null;
    y = data?.measurements[index]?.y ?? null;
  }
  async function load(): Promise<void> {
    data = await api.correction(mode);
    choose(selected);
  }
  async function run(action: () => Promise<void>): Promise<void> {
    if (busy) return;
    busy = true;
    error = '';
    saved = '';
    try {
      await action();
    } catch (e) {
      error = explain(e);
    } finally {
      busy = false;
    }
  }
  $effect(() => {
    const laser = mode;
    let live = true;
    data = null;
    error = '';
    selected = 0;
    api.correction(laser)
      .then(value => {
        if (live) {
          data = value;
          choose(0);
        }
      })
      .catch(e => {
        if (live) error = explain(e);
      });
    return () => {
      live = false;
    };
  });

  async function saveCell(): Promise<void> {
    if (!data || x === null || y === null) return;
    const measurements = [...data.measurements] as CorrectionView['measurements'];
    measurements[selected] = { x, y };
    await api.saveCorrection({ action: 'measure', mode, revision: data.revision, measurements, apply: false });
    await load();
    saved = `Square ${selected + 1} saved`;
  }
  function measure(axis: 'x' | 'y'): void {
    osk.number(`Square ${selected + 1} · ${axis.toUpperCase()} measurement`, (axis === 'x' ? x : y) ?? 100, 'mm', value => {
      if (!Number.isFinite(value) || value < 90 || value > 110) {
        error = `Enter a measurement between ${distance(90)} and ${quantity(110, 'mm')}.`;
        return;
      }
      if (axis === 'x') x = value; else y = value;
      void run(saveCell);
    });
  }
  async function apply(): Promise<void> {
    if (!data) return;
    await api.saveCorrection({ action: 'measure', mode, revision: data.revision, measurements: data.measurements, apply: true });
    await load();
    saved = 'Correction saved. New DXF jobs will use this profile.';
  }
  async function enable(): Promise<void> {
    if (!data) return;
    await api.saveCorrection({ action: 'enable', mode, revision: data.revision, enabled: !data.enabled });
    await load();
  }
  function loadCoupon(): Promise<void> {
    return run(async () => {
      await api.correctionCoupon(mode);
      ui.setupPanel = null;
      ui.tab = 'setup';
    });
  }
  function pickSquare(index: number): void {
    choose(index);
    page = 1;
  }
  function nextSquare(): void {
    if (!data) return;
    const next = data.measurements.findIndex((m, i) => !m && i !== selected);
    if (next < 0) page = 2;
    else choose(next);
  }
  async function confirmApply(): Promise<void> {
    const ok = await ui.confirm({
      title: 'Apply this correction?',
      body: `New ${laserLabel(mode)} DXF jobs are cut to these nine measurements. Saved jobs keep the correction they were made with.`,
      confirm: 'Apply to new DXFs',
    });
    if (ok) run(apply);
  }
  function squareLabel(index: number): string {
    return `Square ${index + 1}${data?.measurements[index] ? ', measured' : ', unmeasured'}`;
  }
  function squareSize(index: number): string {
    const m = data?.measurements[index];
    return m ? `${distance(m.x, 2)} × ${quantity(m.y, 'mm', 2)}` : `${distance(100)} × ${quantity(100, 'mm')}`;
  }
  function range(axis: keyof Measurement): string {
    const values = data?.measurements.flatMap(value => value ? [value[axis]] : []) ?? [];
    return values.length ? `${distance(Math.min(...values), 3)} – ${distance(Math.max(...values), 3)} ${unitLabel('mm')}` : '—';
  }
</script>

<div class="calibration">
  <header>
    <p>Nine {distance(100)} × {quantity(100, 'mm')} squares</p>
    <div class="seg" aria-label="Calibration laser">
      {#each ['fiber', 'co2'] as laser}
        <button class:on={mode === laser} disabled={busy} onclick={() => mode = laser as LaserMode}>{laserLabel(laser as LaserMode)}</button>
      {/each}
    </div>
  </header>
  <nav aria-label="Calibration steps">
    {#each steps as step, index}
      <button class:current={page === index} disabled={busy} onclick={() => page = index}><span>{index + 1}</span>{step}</button>
    {/each}
  </nav>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if data}
    <div class="cal-body">
      <div class="bed-wrap">
        <div class="bed-heading">
          <span>{laserLabel(mode)} · {distance(data.bed.max.x - data.bed.min.x)} × {quantity(data.bed.max.y - data.bed.min.y, 'mm')}</span>
          <span>Back of bed · Y+</span>
        </div>
        <div class="bed" role="group" aria-label="Nine calibration squares">
          {#each order as index}
            <button
              class:chosen={page === 1 && selected === index}
              class:measured={!!data.measurements[index]}
              disabled={busy}
              aria-label={squareLabel(index)}
              aria-pressed={page === 1 && selected === index}
              onclick={() => pickSquare(index)}
            >
              <strong>{index + 1}</strong>
              <span>{squareSize(index)}</span>
              <small>X {distance(data.positions[index]![0])} · Y {quantity(data.positions[index]![1], 'mm')}</small>
            </button>
          {/each}
        </div>
        <div class="bed-heading"><span>Front of bed</span><span>X →</span></div>
        <p class="progress">{count} of 9 measured · saved</p>
      </div>
      <div class="step-content">
        {#if page === 0}
          <span class="eyebrow">1 · Cut squares</span>
          <h3>Nine positions. One square each.</h3>
          <p>Use your usual cutting settings.</p>
          <button class="btn btn-primary lg" disabled={busy} onclick={loadCoupon}>Load calibration job</button><button
            class="text-action" disabled={busy} onclick={() => page = 1}>Already cut? Enter measurements →</button>
        {:else if page === 1}
          <span class="eyebrow">2 · Measure</span>
          <h3>Square {selected + 1}</h3>
          <p>Measure the finished square.</p>
          <div class="measurement">
            <button disabled={busy} onclick={() => measure('x')}>
              <span>X · width</span>
              <strong>{x === null ? 'Enter width' : quantity(x, 'mm', 3)}</strong>
            </button>
            <button disabled={busy} onclick={() => measure('y')}>
              <span>Y · height</span>
              <strong>{y === null ? 'Enter height' : quantity(y, 'mm', 3)}</strong>
            </button>
          </div>
          <button class="btn btn-primary lg" disabled={busy || !data.measurements[selected]} onclick={nextSquare}>
            {count === 9 ? 'Review correction' : 'Next unmeasured square'}
          </button>
        {:else}
          <span class="eyebrow">3 · Apply</span>
          <h3>{data.enabled ? 'Correction is enabled' : 'Ready for future DXFs'}</h3>
          <p>Applies to new {laserLabel(mode)} DXF jobs.</p>
          <dl>
            <div><dt>X measurements</dt><dd>{range('x')}</dd></div>
            <div><dt>Y measurements</dt><dd>{range('y')}</dd></div>
            <div><dt>Squares measured</dt><dd>{count} / 9</dd></div>
          </dl>
          <button class="btn btn-primary lg" disabled={busy || count !== 9} onclick={confirmApply}>Save &amp; apply to new DXFs</button>{#if data.active}<button
            class="text-action" disabled={busy} onclick={() => run(enable)}
          >{data.enabled ? 'Pause correction for new jobs' : 'Enable saved profile'}</button>{/if}
        {/if}
        {#if saved}<p class="saved" role="status">{saved}</p>{/if}
      </div>
    </div>
  {:else if !error}<p>Loading calibration…</p>{/if}
</div>

<style>
  .calibration { max-width: 1180px; margin: 0 auto; padding: 0 10px 20px; }
  header { display:flex; align-items:center; justify-content:space-between; gap:24px; margin-bottom:24px; }
  header p { color:var(--ink-3); margin:0; }
  nav { display:flex; gap:8px; margin-bottom:30px; }
  nav button { flex:1; display:flex; align-items:center; gap:12px; padding:12px 18px; min-height:58px; border:1px solid var(--line); border-radius:10px; color:var(--ink-3); background:var(--panel-2); text-align:left; font-size:var(--t-base); cursor:pointer; }
  nav button span { display:grid; place-items:center; width:27px; height:27px; border:1px solid var(--line); border-radius:50%; }
  nav button.current { color:var(--ink); border-color:var(--accent); background:color-mix(in srgb,var(--accent) 8%,var(--panel)); } nav .current span { color:white; background:var(--accent); border-color:var(--accent); }
  .cal-body { display:grid; grid-template-columns:minmax(0,1.3fr) minmax(290px,1fr); gap:38px; align-items:start; }
  .bed { display:grid; grid-template-columns:repeat(3,1fr); gap:18px; padding:22px; border:2px solid var(--line); border-radius:12px; background:var(--panel-2); }
  .bed button { min-height:106px; display:flex; flex-direction:column; justify-content:center; align-items:center; gap:7px; border:1px solid var(--line); border-radius:8px; background:var(--panel); color:var(--ink); cursor:pointer; padding:10px 5px; }
  .bed strong { font-size:var(--t-xl); }
  .bed span { font-size:var(--t-sm); }
  .bed small { font-size:var(--t-sm); color:var(--ink-3); }
  .bed button.measured { border-color:var(--ok); }
  .bed button.chosen { outline:3px solid var(--accent); outline-offset:3px; }
  .bed-heading { display:flex; justify-content:space-between; gap:15px; color:var(--ink-3); font-size:var(--t-sm); padding:10px 0; }
  .progress { color:var(--ink-3); font-size:var(--t-sm); margin:8px 0; } .eyebrow { text-transform:uppercase; letter-spacing:.12em; font-size:var(--t-sm); color:var(--accent); font-weight:700; }
  .step-content { padding-top:18px; }
  h3 { font-size:var(--t-xl); line-height:1.2; margin:12px 0 18px; color:var(--ink); text-transform:none; letter-spacing:normal; }
  .step-content p { color:var(--ink-3); font-size:var(--t-base); line-height:1.7; margin:0 0 18px; }
  .step-content .btn { width:100%; min-height:56px; margin-top:10px; } .text-action { min-height:50px; width:100%; border:0; background:none; color:var(--ink-2); font-size:var(--t-sm); cursor:pointer; margin-top:8px; }
  .measurement { display:grid; grid-template-columns:1fr 1fr; gap:12px; margin:24px 0 14px; }
  .measurement button { min-height:104px; text-align:left; padding:16px; border:1px solid var(--line); border-radius:10px; background:var(--panel-2); color:var(--ink); cursor:pointer; }
  .measurement span { display:block; font-size:var(--t-sm); color:var(--ink-3); margin-bottom:12px; }
  .measurement strong { font-size:var(--t-lg); }
  .step-content dl { margin:25px 0; }
  dl div { display:flex; justify-content:space-between; gap:15px; padding:13px 0; border-bottom:1px solid var(--line); font-size:var(--t-sm); }
  dt { color:var(--ink-3); }
  dd { margin:0; }
  .error { color:var(--warn); }
  .step-content .saved { color:var(--ok); margin-top:16px; }
  header .seg button { min-height:48px; }
  @media(max-width:950px) { .cal-body { gap:22px; } .bed { gap:12px; padding:14px; } .bed small { display:none; } }
  @media(max-width:720px) { .cal-body { grid-template-columns:1fr; } header { align-items:flex-start; } nav button { padding:10px; gap:8px; font-size:var(--t-sm); } }
  @media(max-height:850px) { header { margin-bottom:12px; } nav { margin-bottom:14px; } nav button { min-height:50px; padding-block:8px; } .bed { gap:10px; padding:12px; } .bed button { min-height:80px; gap:4px; padding:7px; } .bed strong { font-size:var(--t-xl); } .bed-heading { padding:8px 0; } .step-content { padding-top:8px; } h3 { font-size:var(--t-xl); margin:10px 0 12px; } .step-content p { font-size:var(--t-sm); margin-bottom:12px; } .measurement { margin:14px 0; } }
</style>
