<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import PreflightEditor from '../components/PreflightEditor.svelte';
  import ManualTests from '../components/ManualTests.svelte';
  import MatrixCalibration from '../components/MatrixCalibration.svelte';
  import MachineSettings from '../components/MachineSettings.svelte';
  import MachineFiles from '../components/MachineFiles.svelte';
  import { SETTINGS_PAGES } from '../lib/navigation';
  import { APP_VERSION } from '../lib/version';
  import { settingsEdits } from '../lib/settings-edits.svelte';
  import { Hold } from '../lib/hold';
  import { access, rememberLayout } from '../lib/access.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain, fmt, laserLabel } from '../lib/format';
  import type { OutputRequest } from '../api';
  import { units, distance, displayNumber, quantity, unitLabel } from '../lib/units.svelte';

  let { compact = false, sectionOnly = false }: { compact?: boolean; sectionOnly?: boolean } = $props();
  const doc = $derived(server.doc!);
  const machine = $derived(doc.machine);
  const feedback = $derived(machine.feedback);
  const bindings = $derived(doc.bindings);
  const connection = $derived(machine.connection);
  const readiness = $derived(doc.readiness);
  const link = $derived(doc.link);

  const page = $derived(SETTINGS_PAGES.find(page => page.id === ui.machinePage) ?? SETTINGS_PAGES[0]!);
  onDestroy(() => { ui.checklistEditor = null; });


  async function run(action: () => Promise<unknown>): Promise<void> {
    try { await action(); } catch (error) { ui.say(explain(error), true); }
  }

  const held = new Hold({
    heartbeat: (lease) => api.machine('heartbeat', { lease }),
    release: (lease) => api.machine('release', { lease }),
  }, (error) => ui.say(explain(error), true), () => server.link && access.canControl);
  onMount(() => held.mount());
  $effect(() => { if (!server.link || !access.canControl) held.cancel(); });
  $effect(() => { void ui.machinePage; return held.cancel; });
  let gasPressure = $state(5);
  function hold(event: PointerEvent, output: OutputRequest): void {
    held.press(event, (lease) => api.machine('outputs', { output, lease }));
  }
  const GAS = ['Low air', 'Low O₂', 'Low N₂', 'High air', 'High O₂', 'High N₂'];
  const inputOn = (input: number): boolean => !!feedback && ((feedback.inputs >> (input - 1)) & 1) === 1;
</script>

<section class="panel main" class:full-settings={page.wide}>
  {#if !sectionOnly}<div class="panel-head settings-head">
    <div class="settings-title"><h1>Settings</h1><span>v{APP_VERSION}</span></div>
    <nav class="settings-navigation" aria-label="Settings sections"><div class="seg">{#each SETTINGS_PAGES as item}<button class:on={ui.machinePage === item.id} aria-current={ui.machinePage === item.id ? 'page' : undefined} onclick={() => (ui.machinePage = item.id)}>{item.label}</button>{/each}</div></nav>
  </div>{/if}
  <div class="settings" class:wide-page={page.wide}>
    {#each page.groups as group}
      {#if group === 'xml'}
        <MachineSettings />
      {:else if group === 'matrix'}
        <MatrixCalibration />
      {:else if group === 'process'}
        <MachineFiles kind="soft" disabled={!!machine.operation} />
      {:else if group === 'checklists'}
        <div class="setting-group"><h3>Checklist defaults</h3>
          <div class="setting"><div class="lbl">Before cutting<small>Fiber and CO₂ · gas confirmation</small></div><button class="btn btn-ghost" onclick={() => (ui.checklistEditor = 'defaults')}>Preflight defaults</button></div>
          <div class="setting"><div class="lbl">When paused<small>Clear debris and check before resuming</small></div><button class="btn btn-ghost" onclick={() => (ui.checklistEditor = 'pause')}>Pause defaults</button></div>
          <div class="setting"><div class="lbl">After job completion<small>Parking and part inspection</small></div><button class="btn btn-ghost" onclick={() => (ui.checklistEditor = 'postflight')}>Postflight defaults</button></div>
        </div>
      {:else if group === 'display'}
        <div class="setting-group"><h3>Display</h3>
          <div class="setting"><div class="lbl">Units</div><div class="seg" role="group" aria-label="Display units"><button class:on={units.system === 'metric'} aria-pressed={units.system === 'metric'} onclick={() => units.set('metric')}>Metric · mm</button><button class:on={units.system === 'imperial'} aria-pressed={units.system === 'imperial'} onclick={() => units.set('imperial')}>Imperial · in</button></div></div>
          <div class="setting"><div class="lbl">Night mode<small>Preview · save in Pending changes</small></div><button class="switch" class:on={ui.theme === 'dark'} onclick={() => run(() => settingsEdits.theme(ui.theme === 'dark' ? 'light' : 'dark'))} aria-label="Night mode"></button></div>
          <div class="setting"><div class="lbl">Local network<small>{access.info?.address.replace('http://', '')}</small></div><button class="btn btn-ghost" onclick={() => access.manage = true}>Open</button></div>
          <div class="setting"><div class="lbl">{compact ? 'Full interface' : 'Phone interface'}</div><a class="btn btn-ghost" href={compact ? '/app' : '/mobile'} onclick={() => rememberLayout(compact ? 'full' : 'mobile')}>Open</a></div>
        </div>
      {:else if group === 'controller'}
        <div class="setting-group"><h3>Controller</h3>
          <div class="setting"><div class="lbl">Link<small>UDP to the motion card</small></div><div class="val">{connection.state === 'connected' ? connection.endpoint : connection.state === 'faulted' ? `lost: ${connection.reason}` : 'disconnected'}</div></div>
          <div class="setting"><div class="lbl">Route<small>Network adapter</small></div><div class="val">{link.adapter ? `${link.adapter.description || link.adapter.name} · ${link.host ?? '—'}` : 'found on connection'}</div></div>
          <div class="setting"><div class="lbl">Last attempt<small>{link.phase === 'failed' ? 'failed' : link.phase === 'idle' ? 'ended' : 'in progress'}</small></div><div class="val">{link.detail || (connection.state === 'connected' ? 'connected' : '—')}</div></div>
          {#if link.choices.length}
            <div class="setting"><div class="lbl">Machine adapter<small>Save before connecting.</small></div><div class="seg">{#each link.choices as choice}<button onclick={() => run(() => settingsEdits.route('adapter', choice.name, link.remembered_adapter))}>{choice.description || choice.name}</button>{/each}</div></div>
          {/if}
          <div class="setting"><div class="lbl">Controller address<small>ip:port · save edits in Pending changes</small></div><button class="val" onclick={() => osk.text('Controller address', settingsEdits.entries.route?.controller?.value ?? link.controller, (v) => run(() => settingsEdits.route('controller', v, link.controller)))}>{settingsEdits.entries.route?.controller?.value ?? link.controller}</button></div>
          <div class="setting"><div class="lbl">This computer's address<small>ip/prefix · save edits in Pending changes</small></div><button class="val" onclick={() => osk.text("This computer's address", settingsEdits.entries.route?.host?.value ?? link.computer, (v) => run(() => settingsEdits.route('host', v, link.computer)))}>{settingsEdits.entries.route?.host?.value ?? link.computer}</button></div>
          <div class="setting"><div class="lbl">Identity<small>product · firmware</small></div><div class="val">{machine.identity ? `${machine.identity.product_id} · ${machine.identity.program_version}` : '—'}</div></div>
          <div class="setting"><div class="lbl">Scale · cycle<small>{unitLabel('units/mm')} · interpolation</small></div><div class="val">{feedback ? `${displayNumber(feedback.scale, 'units/mm')} · ${feedback.cycle_us} µs` : '—'}</div></div>
          <div class="setting"><div class="lbl">Feedback age<small>last complete snapshot</small></div><div class="val">{feedback ? `${feedback.age_ms} ms` : '—'}</div></div>
        </div>
      {:else if group === 'laser'}
        <div class="setting-group"><h3>Laser</h3>
          <div class="setting"><div class="lbl">Operating mode<small>Home after switching</small></div>
            <div class="seg">{#each ['fiber', 'co2'] as mode}<button class:on={doc.mode === mode} disabled={!readiness.mode.ok && doc.mode !== mode} title={readiness.mode.reason ?? ''} onclick={() => { if (doc.mode !== mode) run(() => api.machine('mode', { mode: mode as 'fiber' | 'co2' })); }}>{laserLabel(mode as 'fiber' | 'co2')}</button>{/each}</div></div>
          <div class="setting"><div class="lbl">Applied on this connection<small>Current session</small></div><div class="val">{machine.session.mode ? laserLabel(machine.session.mode) : 'not yet'}</div></div>
          <div class="setting"><div class="lbl">Head controller<small>Machine backup</small></div><div class="val">{bindings ? (bindings.head_enabled ? 'configured' : 'none') : '—'}</div></div>
          <div class="setting"><div class="lbl">Head calibration<small>on this connection</small></div><div class="val">{machine.session.calibration ?? 'not yet'}</div></div>
        </div>
      {:else if group === 'axes'}
        <div class="setting-group"><h3>Axes &amp; travel</h3>
          <div class="setting"><div class="lbl">X travel<small>soft limits</small></div><div class="val">{bindings ? `${distance(bindings.extent[0][0])} to ${quantity(bindings.extent[0][1], 'mm')}` : '—'}</div></div>
          <div class="setting"><div class="lbl">Y travel<small>soft limits</small></div><div class="val">{bindings ? `${distance(bindings.extent[1][0])} to ${quantity(bindings.extent[1][1], 'mm')}` : '—'}</div></div>
          <div class="setting"><div class="lbl">Jog speeds<small>slow · fast</small></div><div class="val">{bindings ? `${displayNumber(bindings.jog_speed[0], 'mm/s')} · ${quantity(bindings.jog_speed[1], 'mm/s')}` : '—'}</div></div>
          <div class="setting"><div class="lbl">Reference<small>Go Origin on this connection</small></div><div class="val">{machine.session.homed ? 'established' : 'required'}</div></div>
          <div class="setting"><div class="lbl">Axis parameters<small>Compared with backup</small></div><div class="val">{machine.session.parameters_verified ? 'read' : 'not read'}</div></div>
          <div class="setting"><div class="lbl">Position<small>machine coordinates</small></div><div class="val">{feedback ? `${feedback.position_mm.map((v) => distance(v)).join(' · ')} ${unitLabel('mm')}` : '—'}</div></div>
        </div>
      {:else if group === 'safety'}
        <div class="setting-group"><h3>Safety I/O</h3>
          {#each bindings?.rules ?? [] as rule}
            <div class="setting"><div class="lbl">{rule.label}<small>input {rule.input} · id {rule.id}</small></div><div class="val">{feedback ? (inputOn(rule.input) ? 'high' : 'low') : '—'}</div></div>
          {:else}
            <div class="setting"><div class="lbl">No input rules<small>{doc.bindings_error ?? 'bind the machine files by connecting'}</small></div></div>
          {/each}
          <div class="setting"><div class="lbl">Outputs<small>standard bank · extended</small></div><div class="val mono">{feedback ? `${feedback.outputs.toString(2).padStart(10, '0')} · ${feedback.extended_outputs.toString(16)}` : '—'}</div></div>
        </div>
      {:else if group === 'outputs'}
        <ManualTests />
        <div class="setting-group"><h3>Manual outputs</h3>
          <div class="setting"><div class="lbl">Hold a button<small>Release to turn off.</small></div></div>
          <div class="setting"><div class="lbl">Pointer<small>{doc.mode === 'fiber' ? 'Fiber' : 'CO₂'} · {bindings?.outputs.pointer_port ? `Output ${bindings.outputs.pointer_port}` : 'Unassigned'}</small></div><button class="btn btn-ghost" disabled={!bindings?.outputs.pointer || !readiness.outputs.ok} onpointerdown={(event) => hold(event, { kind: 'pointer' })}>Hold</button></div>
          <div class="setting"><div class="lbl">Shutter<small>{doc.mode === 'fiber' ? 'Fiber' : 'CO₂'} · {bindings?.outputs.shutter_port ? `Output ${bindings.outputs.shutter_port}` : 'Unassigned'}</small></div><button class="btn btn-ghost" disabled={!bindings?.outputs.shutter || !readiness.outputs.ok} onpointerdown={(event) => hold(event, { kind: 'shutter' })}>Hold</button></div>
          {#each GAS as name, selector}
            {#if bindings?.outputs.gas[selector]}
              <div class="setting"><div class="lbl">{name}<small>valve{selector < 3 ? ' and proportional pressure' : ''}</small></div><button class="btn btn-ghost" disabled={!readiness.outputs.ok} onpointerdown={(event) => hold(event, { kind: 'gas', selector, pressure: gasPressure })}>Hold</button></div>
            {/if}
          {/each}
          <div class="setting"><div class="lbl">Gas pressure<small>for the proportional valve</small></div><button class="val" data-numpad onclick={() => osk.number('Gas pressure', gasPressure, 'bar', (v) => { if (v >= 0 && v <= 100) gasPressure = v; })}>{quantity(gasPressure, 'bar')}</button></div>
        </div>
      {/if}
    {/each}
  </div>
</section>

{#if !page.wide && !sectionOnly}<aside class="panel side">
  <h2>Diagnostics</h2>
  <div class="checklist">
    <div class="check" class:ok={server.link} class:bad={!server.link}><span class="mark"></span>Server event stream</div>
    <div class="check" class:ok={connection.state === 'connected'} class:bad={connection.state !== 'connected'}><span class="mark"></span>Controller link</div>
    <div class="check" class:ok={!!bindings} class:bad={!bindings}><span class="mark"></span>{bindings ? `Machine files bound · ${laserLabel(bindings.mode)}` : doc.bindings_error ?? 'Machine files not bound'}</div>
    <div class="check" class:ok={machine.session.homed} class:bad={!machine.session.homed}><span class="mark"></span>Axes homed</div>
    <div class="check" class:ok={machine.alarms.length === 0} class:bad={machine.alarms.length > 0}><span class="mark"></span>{machine.alarms.length === 0 ? 'No alarms' : `${machine.alarms.length} alarm${machine.alarms.length === 1 ? '' : 's'}`}</div>
  </div>

  <div class="field"><h3>Color guide</h3>
    <div class="legend">
      <div><span class="sw" style="background:var(--move-soft);border:1px solid var(--move-line)"></span>Green · motion</div>
      <div><span class="sw" style="background:var(--start)"></span>Dark green · Start</div>
      <div><span class="sw" style="background:var(--stop)"></span>Red · Stop</div>
      <div><span class="sw" style="background:var(--hold-soft);border:1px solid #efd9ad"></span>Amber · hold, alarms, caution</div>
      <div><span class="sw" style="background:var(--accent)"></span>Orange · selection and primary actions</div>
    </div>
  </div>

  <div class="side-foot">
    <button class="btn btn-warn lg block" onclick={() => run(() => api.machine('relieve', { alarm: null }))} disabled={connection.state !== 'connected'}>Reset controller alarms</button>
    <button class="btn btn-stop lg block" onclick={() => run(() => api.machine('stop'))} disabled={!readiness.stop.ok}>Stop motion</button>
  </div>
</aside>{/if}

{#if ui.checklistEditor}<PreflightEditor scope={ui.checklistEditor} onclose={() => (ui.checklistEditor = null)} />{/if}

<style>
  .full-settings { grid-column: 1 / -1; }
  .settings-head { justify-content: flex-start; gap: 20px; padding-block: 12px; }
  .settings-head h1 { flex: none; }
  .settings-title { flex: none; }
  .settings-title span { display: block; margin-top: 3px; font-size: 10px; color: var(--ink-3); }
  .settings-navigation { min-width: 0; overflow-x: auto; }
  .settings-navigation .seg button { min-height: 44px; padding-inline: 13px; }
  .settings.wide-page { display: block; overflow-y: auto; min-height: 0; }
  @media (max-width: 700px) { .settings-head { flex-wrap: wrap; gap: 10px; } .settings-navigation { width: 100%; } }
</style>
