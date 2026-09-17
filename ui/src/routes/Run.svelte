<script lang="ts">
  import { diagnosticText } from '../lib/units.svelte';
  import { distance, quantity, unitLabel } from '../lib/units.svelte';
  // The run page: the compiled program on the bed with its layers, the head,
  // progress from the controller's item tag, and the three controls.
  import { untrack } from 'svelte';
  import SheetStrip from '../components/SheetStrip.svelte';
  import RemnantInspector from '../components/RemnantInspector.svelte';
  import Recovery from '../components/Recovery.svelte';
  import RunSide from './RunSide.svelte';
  import RunControls from '../components/RunControls.svelte';
  import FlightChecklist from '../components/FlightChecklist.svelte';
  import type { ExecutionView, PreflightReview } from '../api';
  import Stage from '../components/Stage.svelte';
  import { runView } from '../lib/run-views.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { frameOf } from '../lib/frame';
  import { LAYERS } from '../lib/layers';
  import { explain, laserLabel, recipeLabel, seconds } from '../lib/format';
  import { boxOf, boxOfBounds, pathOf } from '../lib/svg';
  import { restartAt } from '../lib/recovery';
  import { beginRun } from '../lib/run-actions';
  import { cutHistory } from '../lib/cut-history';

  let { compact = false, recoveryEditor = false }: { compact?: boolean; recoveryEditor?: boolean } = $props();
  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft);
  const machine = $derived(doc.machine);
  const program = $derived(machine.program);
  const execution = $derived(doc.execution);
  const paused = $derived(program?.state === 'held');
  const canRecover = $derived(!!doc.recovery && ['held', 'stopped', 'failed'].includes(doc.recovery.state));
  let showRecovery = $state(false);
  const recovering = $derived(canRecover && (showRecovery || recoveryEditor));
  $effect(() => { if (!canRecover) showRecovery = false; });
  const restartPosition = $derived(recovering ? doc.recovery?.position : paused ? doc.recovery?.pause_position : null);
  let original = $state<ExecutionView | null>(null);
  $effect(() => {
    const id = doc.recovery?.id;
    if (id && original?.id !== id) { untrack(() => {
      if (execution?.id === id) original = execution;
      else api.recoveryProgram().then(reply => { if (server.doc?.recovery?.id === reply.execution.id) original = reply.execution; }).catch(error => ui.say(explain(error), true));
    }); }
  });
  const retained = $derived(execution && !execution.frame && original?.id === doc.recovery?.id ? original : null);
  const displayed = $derived(retained ?? execution);
  const compiled = $derived(displayed?.compiled ?? (recovering ? null : draft?.compiled) ?? null);
  const history = $derived(retained && doc.recovery ? cutHistory(retained.compiled.moves, doc.recovery.steps) : []);
  const resumed = $derived(!!retained && !!execution && retained.id !== execution.id && !recovering);
  const material = $derived(displayed ? displayed.material : draft?.recipe);
  const preview = $derived(draft?.preview ?? null);
  const part = $derived(doc.library.parts.find((p) => p.id === draft?.part) ?? null);
  const jobName = $derived(displayed?.name ?? doc.library.jobs.find((j) => j.id === draft?.job)?.name ?? part?.name ?? 'untitled');
  const frame = $derived.by(() => {
    const frame = frameOf(doc);
    if (displayed) return { ...frame, zero: displayed.zero, origin: displayed.origin };
    if (draft?.placement.mode === 'head' && !draft.placement.captured && frame.head && draft.dock) {
      return { ...frame, origin: frame.head, zero: [frame.head[0] - draft.dock[0], frame.head[1] - draft.dock[1]] as [number, number] };
    }
    return frame;
  });

  const camera = $derived(runView(`${draft?.job ?? draft?.part ?? 'empty'}/${draft?.sheets?.active ?? 0}`));
  const view = $derived(camera.view);
  const box = $derived(compiled ? boxOf(compiled.moves.filter(m => m.kind !== 'travel').map((m) => m.points)) : preview?.bounds ? boxOfBounds(preview.bounds) : null);
  const fit = () => {
    view.setLimit(frame.bed);
    if (box) view.fit({ minX: box.minX + frame.zero[0], maxX: box.maxX + frame.zero[0], minY: box.minY + frame.zero[1], maxY: box.maxY + frame.zero[1] });
    else if (frame.bed) view.home();
    camera.fitted = true;
  };
  $effect(() => {
    view.setLimit(frame.bed);
    if (!camera.fitted && (frame.bed || box)) {
      untrack(() => { if (frame.bed) { view.home(); camera.fitted = true; } else fit(); });
    }
  });

  const running = $derived(program?.state === 'running' || program?.state === 'finishing');
  /** Item tags name the remainder; map them back to the full retained job. */
  const contours = $derived(compiled?.plan.length ?? 0);
  const done = $derived(retained ? doc.recovery?.steps.filter(s => s.status === 'completed').length ?? 0 : execution ? doc.progress?.completed ?? 0 : 0);
  const current = $derived.by(() => {
    if (recovering) return doc.recovery?.selected?.pass ?? null;
    const index = doc.progress?.pass;
    if (!execution || index == null) return null;
    if (!retained) return index;
    const ordinal = execution.compiled.plan[index]?.ordinal;
    const root = compiled?.plan.findIndex(pass => pass.ordinal === ordinal) ?? -1;
    return root < 0 ? null : root;
  });
  const pass = $derived(current === null ? null : compiled?.plan[current] ?? null);
  const cutting = $derived(running && !doc.progress?.approaching && pass?.kind !== 'pre_pierce' ? current : null);
  const pct = $derived(contours > 0 ? (done / contours) * 100 : 0);
  const eta = $derived(recovering ? 'Original program' : execution ? seconds(execution.compiled.seconds * (1 - (doc.progress?.completed ?? 0) / Math.max(1, execution.compiled.plan.length))) : compiled ? seconds(compiled.seconds) : '—');
  /** The next travel move, when only that one is shown. */
  const nextTravel = $derived(compiled?.moves.findIndex((m) => m.kind === 'travel' && (m.pass ?? 0) >= done) ?? -1);
  const resumeTravels = $derived(resumed ? execution!.compiled.moves.filter(move => move.kind === 'travel' && (move.pass ?? 0) >= (doc.progress?.completed ?? 0)) : []);

  const message = $derived.by(() => {
    if (recovering) return doc.can_resume ? 'Resume starts at the selected recovery point.' : 'Choose and prepare the restart point.';
    if (paused) return machine.operation?.kind === 'program' ? 'Pausing · waiting for the head to stop.' : doc.can_resume ? 'Paused · Resume returns to the saved position.' : doc.recovery?.problem ?? 'Paused · saving the position.';
    if (execution && running) {
      if (execution.frame) return 'Framing · laser off.';
      if (compiled?.dry_run) return 'Dry run · laser off.';
      if (!program?.started) return 'Preparing to run · keep clear of the bed.';
      const phase = doc.progress?.approaching ? 'Travel' : pass?.kind === 'pre_pierce' ? 'Pre-piercing' : pass?.kind === 'film' ? 'Film removal' : 'Cutting process';
      return `${phase}${pass ? ` · pass ${pass.ordinal + 1}` : ''} · keep clear of the bed.`;
    }
    if (execution && program?.state === 'completed') return execution.frame ? 'Frame finished.' : compiled?.dry_run ? 'Dry run finished.' : 'Program finished. Check the cut.';
    if (execution && program?.state === 'stopped') return 'Job stopped.';
    return doc.readiness.run.ok ? 'Press Start.' : diagnosticText(doc.readiness.run.reason ?? '');
  });

  let remnant = $state<string | null>(null);
  let showLayers = $state(false);
  let preflight = $state<PreflightReview | null>(null);
  let reviewing = $state(false);
  let choosingRun = $state(false);
  async function chooseRun(dryRun: boolean): Promise<void> {
    if (choosingRun || draft?.dry_run === dryRun) return;
    choosingRun = true;
    try { await api.compile(dryRun); }
    catch (error) { ui.say(explain(error), true); }
    finally { choosingRun = false; }
  }
  async function act(action: 'run' | 'resume' | 'hold' | 'stop'): Promise<void> {
    try {
      if (action === 'run' || action === 'resume') {
        if (reviewing) return;
        reviewing = true;
        preflight = await beginRun(action, doc);
      } else await api.machine(action);
    } catch (error) { ui.say(explain(error), true); }
    finally { reviewing = false; }
  }
  const mark = $derived(Math.min(view.w, view.h) / 120);
  let pickingRestart = false;
  async function pickRestart(point: [number, number]): Promise<void> {
    if (!recovering || !compiled || !doc.recovery || machine.operation || pickingRestart) return;
    const choice = restartAt(compiled, doc.recovery.steps, [point[0] - frame.zero[0], point[1] - frame.zero[1]], 24 * view.mmPerPixel);
    if (!choice) return;
    pickingRestart = true;
    try { await api.recoveryChange({ kind: 'select', ...choice }, doc.recovery.revision); }
    catch (error) { ui.say(explain(error), true); }
    finally { pickingRestart = false; }
  }
</script>

<section class="panel main run-panel">
  <div class="panel-head compact">
    <div class="crumb">
      <strong>{jobName}</strong>
      <span class="run-msg run-material">{material ? `${recipeLabel(material)} · ${laserLabel(material.laser)}` : 'No material'}</span>
      {#if draft?.calibration}<span class="run-msg run-correction">Correction off</span>{:else if !displayed && draft?.placement.correction_pending}<span class="run-msg run-correction">Correction pending position</span>{/if}
    </div>
    <div class="run-meta">
      <strong>{execution?.frame ? 'Frame' : `${Math.round(pct)}%`}</strong>
      <span>{execution?.frame ? 'Laser off' : `${done} / ${contours} passes`}</span>
      {#if !execution?.frame}<span>{execution && !recovering && program?.state === 'completed' ? 'done' : eta}</span>{/if}
    </div>
    <div class="actions"><span class="muted">{message}</span></div>
  </div>

  {#if compiled?.plan.some((pass) => pass.omitted_cooling > 0)}
    <p class="muted">{compiled.plan.reduce((n, pass) => n + pass.omitted_cooling, 0)} cooling points skipped within {quantity(0.2, 'mm')} of endpoints.</p>
  {/if}

  {#if !recovering && !running && !paused}<SheetStrip />{/if}
  {#if draft && !recovering && !running && !paused}
    <div class="run-choice">
      <span class="muted">{draft.error ?? (choosingRun ? 'Preparing…' : draft.dry_run ? 'Laser off' : '')}</span>
      <div class="seg">
        <button class:on={!draft.dry_run} disabled={choosingRun || !doc.readiness.compile.ok || !!machine.operation} onclick={() => chooseRun(false)}>Cut</button>
        <button class:on={draft.dry_run} disabled={choosingRun || !doc.readiness.compile.ok || !!machine.operation} onclick={() => chooseRun(true)}>Dry run</button>
      </div>
    </div>
  {/if}
  <Stage {view} variant="run-canvas" bed={frame.bed} head={frame.head} origin={frame.origin} ontap={pickRestart}>
    {#if draft?.stock_outline.length && !displayed}
      <g transform="translate({frame.zero[0]} {frame.zero[1]})">
        <path class="stock-reference" d={pathOf(draft.stock_outline, false) + 'Z'} vector-effect="non-scaling-stroke" />
        {#each draft.stock_cutouts as cutout}<path class="stock-cutout" d={pathOf(cutout, false) + 'Z'} vector-effect="non-scaling-stroke" />{/each}
      </g>
    {/if}
    <g transform="translate({frame.zero[0]} {frame.zero[1]})">
    {#if compiled}
      {#each compiled.moves as move, i}
        {@const finished = move.pass !== null && (retained ? doc.recovery?.steps[move.pass]?.status === 'completed' : move.pass < done)}
        {#if ui.layerShown(finished ? 'done' : move.kind) && (move.kind !== 'travel' || !resumed && (ui.travelMode === 'all' || i === nextTravel))}
          <path class="path {move.kind}" class:done={finished} class:restart-selected={recovering && move.pass !== null && move.pass === current && move.kind !== 'travel'} class:skipped={recovering && move.pass !== null && doc.recovery?.steps[move.pass]?.status === 'skipped'} class:active={move.pass !== null && move.pass === cutting} d={pathOf(move.points, false)} vector-effect="non-scaling-stroke"/>
        {/if}
      {/each}
      {#if ui.layerShown('travel')}{#each ui.travelMode === 'all' ? resumeTravels : resumeTravels.slice(0, 1) as move}<path class="path travel" d={pathOf(move.points, false)} vector-effect="non-scaling-stroke" />{/each}{/if}
      {#if ui.layerShown('done')}{#each history as points}<path class="path done" d={pathOf(points, false)} vector-effect="non-scaling-stroke" />{/each}{/if}
      {#if ui.layerShown('pierce')}{#each compiled.pierces as [x, y]}<circle class="mark pierce" cx={x} cy={y} r={mark * 1.4}/>{/each}{/if}
    {:else if preview}
      {#each preview.contours as contour}{#each contour.paths as path}<path class="path {path.kind}" d={pathOf(path.points, false)} vector-effect="non-scaling-stroke"/>{/each}{/each}
    {/if}
    </g>
    {#if restartPosition}<g aria-label={recovering ? 'Selected restart' : 'Paused position'}><circle cx={restartPosition[0]} cy={restartPosition[1]} r={mark * 2} fill="var(--hold)" stroke="var(--ink)" vector-effect="non-scaling-stroke" /><path d="M{restartPosition[0] - mark * 4} {restartPosition[1]}h{mark * 8} M{restartPosition[0]} {restartPosition[1] - mark * 4}v{mark * 8}" stroke="var(--ink)" vector-effect="non-scaling-stroke" /></g>{/if}
    {#snippet overlay()}
      <div class="canvas-hud">{#if frame.head}<span>Head X {distance(frame.head[0] - frame.zero[0])}</span><span>Y {distance(frame.head[1] - frame.zero[1])}</span><span class="muted">job coordinates · {unitLabel('mm')}</span>{:else}<span>Connect for the head position</span>{/if}<span class="sep"></span><span>{view.percent}%</span></div>
      <div class="canvas-zoom"><button onclick={fit} title="Fit job"><i class="ic ic-fit"></i></button><button onclick={() => view.zoom(1.25)} title="Zoom in"><i class="ic ic-plus"></i></button><button onclick={() => view.zoom(0.8)} title="Zoom out"><i class="ic ic-minus"></i></button></div>
    {/snippet}
  </Stage>
  <div class="legend-bar">
    {#if canRecover && !recoveryEditor}<button class="legend-chip recovery-toggle" onclick={() => showRecovery = !showRecovery}>{recovering ? 'Back to controls' : 'Adjust restart…'}</button>{/if}
    {#if recovering && !showLayers}<span class="recovery-key"><i></i>Selected restart</span><span class="recovery-key finished-key"><i></i>Completed</span><button class="legend-chip" onclick={() => showLayers = true}>Display layers…</button>{:else}
    {#each LAYERS as [layer, label]}<button class="legend-chip" class:off={!ui.layerShown(layer)} onclick={() => ui.toggleLayer(layer)}><svg class="sample" viewBox="0 0 36 12" aria-hidden="true">{#if layer === 'pierce'}<circle class="mark pierce" cx="18" cy="6" r="5"/>{:else}<path class="path {layer}" d="M2 6H34"/>{/if}</svg>{label}</button>{/each}
    {#if recovering}<button class="legend-chip" onclick={() => showLayers = false}>Done</button>{/if}{/if}
    <span class="spacer"></span>
    {#if !recovering || showLayers}<div class="seg small"><button class:on={ui.travelMode === 'next'} onclick={() => ui.setTravelMode('next')}>Next move</button><button class:on={ui.travelMode === 'all'} onclick={() => ui.setTravelMode('all')}>All moves</button></div>{/if}
  </div>

  {#if doc.completed_sheet && !running && !recovering}<div class="sheet-complete"><span><strong>Sheet complete</strong><small>Inspect the parts and remaining material.</small></span><button class="btn btn-ghost" onclick={() => remnant = doc.completed_sheet}>Save remaining sheet</button></div>{/if}
  {#if !recovering}
  <RunControls onaction={act} busy={reviewing} showStop={!compact} />
  {/if}
</section>

{#if !compact || recovering}
  {#if recovering}<Recovery onresume={() => act('resume')} {reviewing} />{:else}<RunSide />{/if}
{/if}
{#if remnant}<RemnantInspector id={remnant} onclose={() => remnant = null} />{/if}
{#if preflight}<FlightChecklist initial={preflight} onclose={() => (preflight = null)} />{/if}

<style>
.run-choice { display:flex; align-items:center; justify-content:flex-end; gap:8px; padding:8px 12px; border-bottom:1px solid var(--line); }
.run-choice > span { margin-right:auto; font-size:12px; }
.run-choice button { min-height:44px; }
.recovery-toggle { min-height:44px; }
.panel-head.compact { display:grid; grid-template-columns:auto minmax(0,1fr); gap:5px 14px; min-height:66px; padding:9px 12px; }
.crumb { grid-column:1 / -1; min-width:0; }
.crumb > strong { flex:none; max-width:40%; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.run-material { flex:1; }
.run-correction { flex:none; }
.run-meta { white-space:nowrap; gap:10px; align-items:center; }
.panel-head > .actions { display:block; min-width:0; text-align:right; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.stock-reference { fill:var(--accent); fill-opacity:.035; stroke:var(--accent); stroke-width:1; }
.stock-cutout { fill:var(--ink-3); fill-opacity:.2; stroke:var(--ink-3); stroke-width:1; }
.path.skipped { opacity:0.25; stroke-dasharray:5 5; } .path.restart-selected { stroke:var(--hold); stroke-width:3; opacity:1; }
.sheet-complete { display:flex; align-items:center; justify-content:space-between; gap:20px; padding:14px 22px; border-top:1px solid var(--line); } .sheet-complete strong { font-size:14px; } .sheet-complete small { display:block; margin-top:5px; color:var(--ink-3); font-size:11px; } .sheet-complete button { min-height:48px; } .recovery-key { display:flex; align-items:center; gap:8px; font-size:11px; color:var(--ink-3); } .recovery-key i { width:22px; height:3px; background:var(--hold); } .finished-key i { background:var(--ink-3); opacity:.45; }
</style>
