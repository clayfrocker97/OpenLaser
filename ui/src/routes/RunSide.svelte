<script lang="ts">
  import { diagnosticText, distance, quantity, unitLabel } from '../lib/units.svelte';
  // The run side: the DRO, homing and origin, and the jog pad. An axis
  // moves while its button is held and a heartbeat renews the lease;
  // anything that could lose the release, a blur, a hidden page or a
  // cancelled pointer, releases too.
  import { onMount } from 'svelte';
  import { Hold } from '../lib/hold';
  import SheetPosition from '../components/SheetPosition.svelte';
  import HoldButton from '../components/HoldButton.svelte';
  import GasLaserCard from '../components/GasLaserCard.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain } from '../lib/format';
  import { access } from '../lib/access.svelte';

  const doc = $derived(server.doc!);
  const machine = $derived(doc.machine);
  const feedback = $derived(machine.feedback);
  const origin = $derived(doc.draft?.origin ?? null);
  const readiness = $derived.by(() => {
    if (access.canControl) return doc.readiness;
    const closed = { ok: false, reason: 'Take control to move the machine' };
    return { ...doc.readiness, home: closed, position: closed, calibrate: closed, jog: closed, xy_jog: [[closed, closed], [closed, closed]], head_up: closed, head_down: closed, frame: closed };
  });
  const headEnabled = $derived(doc.bindings?.head_enabled ?? false);
  const headJog = $derived(doc.bindings?.outputs.head_jog ?? false);
  const fresh = $derived(doc.machine.connection.state === 'connected' && !!feedback && feedback.age_ms <= 1000);
  const homed = $derived(fresh && doc.machine.session.homed && feedback?.referenced.every(Boolean));
  const headHomed = $derived(fresh && feedback?.head.referenced);

  const work = $derived.by(() => {
    if (!feedback) return null;
    const [x, y, z] = feedback.position_mm;
    return origin ? [x - origin[0], y - origin[1], z] : [x, y, z];
  });

  async function call(action: () => Promise<unknown>): Promise<void> {
    try { await action(); } catch (error) { ui.say(explain(error), true); }
  }

  type Axis = 0 | 1 | 'z' | 'w';
  let tableSpeed = $state(5);
  let auxiliary = $state<'z' | 'w'>('z');
  const tableEnabled = $derived(doc.bindings?.table_maximum_mm != null);
  const auxiliaryUp = $derived(auxiliary === 'z' ? headJog && readiness.head_up.ok : tableEnabled && readiness.jog.ok);
  const auxiliaryDown = $derived(auxiliary === 'z' ? headJog && readiness.head_down.ok : tableEnabled && readiness.jog.ok);
  function selectAuxiliary(axis: 'z' | 'w'): void { held.cancel(); auxiliary = axis; }
  const held = new Hold({
    heartbeat: (lease) => api.machine('heartbeat', { lease }),
    release: (lease) => api.machine('release', { lease }),
  }, (error) => ui.say(explain(error), true), () => server.link && access.canControl);
  onMount(() => held.mount());
  $effect(() => { if (!server.link || !access.canControl) held.cancel(); });
  function press(event: PointerEvent, axis: Axis, positive: boolean): void {
    held.press(event, (lease) => axis === 'w' ? api.machine('table', { lease, table: { positive, speed_mm_s: tableSpeed } }) : axis === 'z'
      ? api.machine('outputs', { lease, output: { kind: 'head_jog', up: positive, fast: ui.jogFast } })
      : api.machine('jog', { lease, jog: { axis, positive, step_mm: null, fast: ui.jogFast } }));
  }

  // Why the held motion controls are unavailable, once per reason.
  const originGate = $derived(readiness.position.ok && !origin ? { ok: false, reason: 'set a job origin first' } : readiness.position);
  const motionReasons = $derived.by(() => {
    const gates: Array<[string, { ok: boolean; reason: string | null }]> = [['Home', readiness.home], ['Go origin', originGate], ['Frame', readiness.frame]];
    if (headEnabled) gates.push(['Calibrate', readiness.calibrate]);
    const reasons = new Map<string, string[]>();
    for (const [name, gate] of gates) if (!gate.ok && gate.reason) reasons.set(gate.reason, [...(reasons.get(gate.reason) ?? []), name]);
    return [...reasons].map(([reason, names]) => names.length === gates.length ? reason : `${names.join(', ')}: ${reason}`);
  });

  const canJog = $derived(readiness.xy_jog.some(axis => axis.some(direction => direction.ok)));
  const jogTitle = $derived(canJog ? '' : readiness.jog.reason ?? '');
  const jogSpeed = $derived(doc.bindings?.jog_speed[ui.jogFast ? 1 : 0]);
</script>

{#snippet key(axis: Axis, positive: boolean, icon: string, label: string, enabled: boolean)}
  <button onpointerdown={(event) => press(event, axis, positive)} disabled={!enabled}><i class="ic {icon}"></i><small>{label}</small></button>
{/snippet}

<aside class="panel side run-side">
 <div class="run-side-scroll">
  <div class="setup-state" role="status" aria-label="Machine setup">
    <span><small>Source</small><strong>{doc.mode === 'fiber' ? 'Fiber' : doc.mode === 'co2' ? 'CO₂' : 'Not selected'}</strong></span>
    <span class:ready={homed}><small>XY reference</small><strong>{homed ? 'Homed' : 'Home needed'}</strong></span>
    <span class:ready={headHomed}><small>Z reference</small><strong>{!headEnabled ? 'Not used' : headHomed ? 'Homed' : 'Home needed'}</strong></span>
    <span class:ready={doc.calibration.current} title={doc.calibration.quality ?? 'Height calibration'}><small>Calibration</small><strong>{!headEnabled ? 'Not used' : doc.calibration.current ? 'Calibrated' : doc.calibration.quality ? 'Material changed' : 'Needed'}</strong></span>
  </div>
  {#if machine.program?.state === 'held' && doc.recovery?.pause_position}
    <div class="pause-position" role="status"><strong>Paused at X {distance(doc.recovery.pause_position[0])} · Y {distance(doc.recovery.pause_position[1])} {unitLabel('mm')}</strong><span>You can move the head. Resume returns to this saved position.</span></div>
  {/if}
  {#if !machine.program || !['running', 'finishing', 'held'].includes(machine.program.state)}<SheetPosition compact />{/if}
  <div class="dro">
    {#each [['X', 0], ['Y', 1], ['Z', 2]] as [name, i]}
      {@const reading = work ? distance(work[i as number]!) : '—'}
      <div class="cell" class:off={!work}>
        <div class="ax"><span>{name} <small>{unitLabel('mm')}</small></span></div>
        <div class="val" style:--characters={Math.max(6, reading.length)}>{reading}</div>
        <div class="mach">{#if feedback}M {distance(feedback.position_mm[i as number]!)}{:else}—{/if}</div>
      </div>
    {/each}
  </div>

  <div class="row motion-actions">
    <HoldButton class="btn btn-move" onhold={() => call(() => api.machine('home'))} disabled={!readiness.home.ok} title={readiness.home.reason ?? ''}><i class="ic ic-home"></i>Home</HoldButton>
    <HoldButton class="btn btn-move" onhold={() => call(() => api.machine('go-origin', { fast: ui.jogFast }))} disabled={!originGate.ok} title={originGate.reason ?? ''}>Go origin</HoldButton>
    <HoldButton class="btn btn-move" label="Calibrate head" onhold={() => call(() => api.machine('calibrate'))} disabled={!readiness.calibrate.ok} title={readiness.calibrate.reason ?? ''}>Calibrate</HoldButton>
  </div>
  <HoldButton class="btn btn-move frame-action" onhold={() => call(() => api.machine('frame'))} disabled={!readiness.frame.ok} title={readiness.frame.reason ?? ''}><i class="ic ic-frame"></i>Frame · laser off</HoldButton>
  {#each motionReasons as reason}<p class="gate-reason motion-reason">{diagnosticText(reason)}</p>{/each}

  <div class="jogblock">
    <div class="jog" class:disabled={!canJog} title={jogTitle}>
      <span></span>{@render key(1, true, 'ic-arrow-up', 'Y+', readiness.xy_jog[1]![1]!.ok)}<span></span>
      {@render key(0, false, 'ic-arrow-left', 'X−', readiness.xy_jog[0]![0]!.ok)}
      <button class="hub" onclick={() => (ui.jogFast = !ui.jogFast)} title="Tap to switch speed"><strong>{ui.jogFast ? 'Fast' : 'Slow'}</strong><small>{jogSpeed != null ? quantity(readiness.xy_recovery ? Math.min(1, jogSpeed) : jogSpeed, 'mm/s') : 'tap to switch'}</small></button>
      {@render key(0, true, 'ic-arrow-right', 'X+', readiness.xy_jog[0]![1]!.ok)}
      <span></span>{@render key(1, false, 'ic-arrow-down', 'Y−', readiness.xy_jog[1]![0]!.ok)}<span></span>
    </div>
    <div class="zcol">
      {@render key(auxiliary, true, 'ic-arrow-up', auxiliary === 'z' ? 'Z up' : 'W+', auxiliaryUp)}
      <button class="hub" onclick={() => selectAuxiliary(auxiliary === 'z' ? 'w' : 'z')} aria-label={auxiliary === 'z' ? 'Z head; switch to W table' : 'W table; switch to Z head'} title={auxiliary === 'z' ? 'Tap for W table' : 'Tap for Z head'}><strong>{auxiliary.toUpperCase()}</strong><small>{auxiliary === 'z' ? 'Head' : 'Table'}</small><small>Tap for {auxiliary === 'z' ? 'W' : 'Z'}</small></button>
      {@render key(auxiliary, false, 'ic-arrow-down', auxiliary === 'z' ? 'Z down' : 'W−', auxiliaryDown)}
    </div>
  </div>
  {#if !canJog && jogTitle && !readiness.xy_recovery}<p class="gate-reason motion-reason">Jog: {diagnosticText(jogTitle)}</p>{/if}
  {#if readiness.xy_recovery}<p class="muted auxiliary-hint" role="status">X/Y limit recovery: hold an available direction to move away up to 1 mm at 1 mm/s or slower. Release between presses. Once clear, use Home.</p>{/if}
  {#if auxiliary === 'w'}
    {#if tableEnabled}
      <div class="table-readout"><span>W <strong>{quantity(feedback?.table_mm, 'mm')}</strong></span><button class="btn btn-ghost" onclick={() => osk.number('Table speed', tableSpeed, 'mm/s', v => { if (v > 0 && v <= 100) tableSpeed = v; })}>{quantity(tableSpeed, 'mm/s')}</button></div>
    {:else}<p class="muted auxiliary-hint">W table motion is not enabled in the loaded machine configuration.</p>{/if}
  {:else if !headEnabled}<p class="muted auxiliary-hint">No head controller configured.</p>
  {:else if readiness.head_recovery}<p class="muted auxiliary-hint" role="status">Z limit recovery: hold the available direction to move up to 1 mm at 1 mm/s or slower. Release between presses. Once clear, use Home.</p>{/if}
  {#if doc.draft}<GasLaserCard />{/if}
 </div>
</aside>

<style>
  .run-side { padding:12px; }
  .setup-state { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:5px; }
  .setup-state > span { display:flex; flex-direction:column; gap:3px; min-width:0; padding:6px; border:1px solid var(--line); border-radius:9px; }
  .setup-state small { color:var(--ink-3); font-size:9px; }
  .setup-state strong { font-size:11px; }
  .setup-state .ready strong { color:var(--accent); }
  .pause-position { display:flex; flex-direction:column; gap:5px; padding:10px; border:1px solid var(--hold); border-radius:9px; font-size:12px; }
  .pause-position span { color:var(--ink-3); line-height:1.4; }
  .run-side-scroll { flex:1; min-height:0; overflow-y:auto; display:flex; flex-direction:column; gap:8px; padding-right:2px; }
  .run-side-scroll > :global(*) { flex-shrink:0; }
  .motion-actions :global(.btn) { min-width:0; padding:0 6px; font-size:12px; }
  .motion-reason { margin:0; font-size:11px; }
  .dro .cell { container-type:inline-size; padding:6px 8px; height:76px; gap:1px; }
  .dro .val { font-size:min(24px, calc(100cqw / var(--characters) / .64)); line-height:1.15; overflow:visible; white-space:nowrap; }
  .dro .mach { font-size:10px; line-height:1.2; color:var(--ink-3); text-align:right; font-variant-numeric:tabular-nums; white-space:nowrap; }
  .run-side-scroll :global(.frame-action) { width:100%; min-height:52px; font-size:15px; }
  .jogblock { flex:1 0 204px; display: grid; grid-template-columns: minmax(0, 3fr) minmax(88px, 1fr); gap: 12px; }
  .jog { height: auto; grid-template-rows:repeat(3,minmax(64px,1fr)); }
  .zcol { grid-template-columns: minmax(0, 1fr); grid-template-rows:repeat(3,minmax(64px,1fr)); }
  .zcol > * { grid-column: 1; }
  .auxiliary-hint { font-size: var(--t-xs); color: var(--ink-3); }
  .table-readout { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
</style>
