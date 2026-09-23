<script lang="ts">
  // Settings: one sidebar of sections with search, Advanced last. Display,
  // checklist, route and gas settings save as they change; only what writes
  // the machine (the backup, process INI and controller parameters) waits in
  // Pending changes for a confirmation. Machine tests and the mode switch live
  // beside the jog pad (components/MachineTools.svelte), not here.
  import { onDestroy, tick } from 'svelte';
  import PreflightEditor from '../components/PreflightEditor.svelte';
  import MatrixCalibration from '../components/MatrixCalibration.svelte';
  import MachineSettings from '../components/MachineSettings.svelte';
  import MachineFiles from '../components/MachineFiles.svelte';
  import GasCosts from '../components/GasCosts.svelte';
  import { holdSeconds, MAX_HOLD_MS, MIN_HOLD_MS } from '../lib/hold-confirm';
  import { SETTINGS_PAGES } from '../lib/navigation';
  import { APP_VERSION } from '../lib/version';
  import { settingsEdits } from '../lib/settings-edits.svelte';
  import { access, rememberLayout } from '../lib/access.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain, laserLabel } from '../lib/format';
  import { units, distance, displayNumber, quantity, unitLabel } from '../lib/units.svelte';

  let { compact = false, sectionOnly = false }: { compact?: boolean; sectionOnly?: boolean } = $props();
  const doc = $derived(server.doc!);
  const machine = $derived(doc.machine);
  const feedback = $derived(machine.feedback);
  const bindings = $derived(doc.bindings);
  const connection = $derived(machine.connection);
  const link = $derived(doc.link);

  const page = $derived(SETTINGS_PAGES.find(page => page.id === ui.machinePage) ?? SETTINGS_PAGES[0]!);
  const query = $derived(ui.settingsQuery.trim().toLocaleLowerCase());
  onDestroy(() => { ui.checklistEditor = null; });

  async function run(action: () => Promise<unknown>, done?: string): Promise<void> {
    try { await action(); if (done) ui.say(done); } catch (error) { ui.say(explain(error), true); }
  }

  // Hold times are one setting for every screen; they save at once.
  function holdTime(key: 'move_ms' | 'zero_ms', label: string): void {
    osk.number(label, doc.hold[key] / 1000, 's', (seconds) => {
      const ms = Math.round(seconds * 1000);
      if (ms < MIN_HOLD_MS || ms > MAX_HOLD_MS) { ui.say(`Use ${holdSeconds(MIN_HOLD_MS)} to ${holdSeconds(MAX_HOLD_MS)}.`, true); return; }
      const saved = $state.snapshot(doc.hold);
      void run(() => api.saveHoldTimes({ ...saved, [key]: ms }, saved), 'Hold time saved.');
    });
  }

  // The controller route saves at once; it is used from the next connection.
  function route(key: 'adapter' | 'controller' | 'host', value: string, saved: string): void {
    if (value === saved) return;
    void run(() => api.setRoute({ [key]: value, expected: { [key]: saved } }), 'Saved · used from the next connection.');
  }

  const inputOn = (input: number): boolean => !!feedback && ((feedback.inputs >> (input - 1)) & 1) === 1;

  // Search shows every section; rows and groups that do not match hide.
  let content = $state<HTMLElement | null>(null);
  let matches = $state(0);
  $effect(() => {
    void query;
    void doc;
    if (!content) return;
    void tick().then(() => { if (content) matches = filterRows(content, query); });
  });

  function filterRows(root: HTMLElement, text: string): number {
    let shown = 0;
    for (const block of root.querySelectorAll<HTMLElement>('.search-block')) {
      const blockMatches = !!text && (block.dataset['keywords'] ?? '').toLocaleLowerCase().includes(text);
      const rows = [...block.querySelectorAll<HTMLElement>('.setting')];
      let any = false;
      for (const row of rows) {
        const group = row.closest<HTMLElement>('.setting-group');
        const title = group?.querySelector('h3')?.textContent?.toLocaleLowerCase() ?? '';
        const hit = !text || blockMatches || title.includes(text) || (row.textContent ?? '').toLocaleLowerCase().includes(text);
        row.hidden = !hit;
        any ||= hit;
      }
      for (const group of block.querySelectorAll<HTMLElement>('.setting-group')) {
        group.hidden = !!text && group.querySelectorAll('.setting').length > 0 && !group.querySelector('.setting:not([hidden])');
      }
      // Components without rows match on their whole text.
      const visible = !text || any || blockMatches || (!rows.length && (block.textContent ?? '').toLocaleLowerCase().includes(text));
      block.hidden = !visible;
      if (visible) shown += 1;
    }
    return shown;
  }
</script>

{#snippet group(name: string)}
  {#if name === 'display'}
    <div class="setting-group"><h3>Display</h3>
      <div class="setting"><div class="lbl">Units</div><div class="seg" role="group" aria-label="Display units"><button class:on={units.system === 'metric'} aria-pressed={units.system === 'metric'} onclick={() => units.set('metric')}>Metric · mm</button><button class:on={units.system === 'imperial'} aria-pressed={units.system === 'imperial'} onclick={() => units.set('imperial')}>Imperial · in</button></div></div>
      <div class="setting"><div class="lbl">Night mode<small>Dark colours for this screen</small></div><button class="switch" role="switch" class:on={ui.theme === 'dark'} aria-checked={ui.theme === 'dark'} aria-label="Night mode" onclick={() => run(() => settingsEdits.saveTheme(ui.theme === 'dark' ? 'light' : 'dark'))}></button></div>
      <div class="setting"><div class="lbl">Hold to move or fire<small>Every screen</small></div><button class="val" data-numpad onclick={() => holdTime('move_ms', 'Hold to move or fire')}>{holdSeconds(doc.hold.move_ms)}</button></div>
      <div class="setting"><div class="lbl">Hold to set origin<small>Every screen</small></div><button class="val" data-numpad onclick={() => holdTime('zero_ms', 'Hold to set origin')}>{holdSeconds(doc.hold.zero_ms)}</button></div>
    </div>
  {:else if name === 'checklists'}
    <div class="setting-group"><h3>Checklists</h3>
      <div class="setting"><div class="lbl">Before cutting<small>Preflight · Fiber and CO₂</small></div><button class="btn btn-ghost" onclick={() => (ui.checklistEditor = 'defaults')}>Edit</button></div>
      <div class="setting"><div class="lbl">When paused<small>Clear debris and check before resuming</small></div><button class="btn btn-ghost" onclick={() => (ui.checklistEditor = 'pause')}>Edit</button></div>
      <div class="setting"><div class="lbl">After the job<small>Postflight · parking and part inspection</small></div><button class="btn btn-ghost" onclick={() => (ui.checklistEditor = 'postflight')}>Edit</button></div>
    </div>
  {:else if name === 'interface'}
    <div class="setting-group"><h3>This screen</h3>
      <div class="setting"><div class="lbl">{compact ? 'Full interface' : 'Phone interface'}<small>The layout also follows the window size</small></div><a class="btn btn-ghost" href={compact ? '/app' : '/mobile'} onclick={() => rememberLayout(compact ? 'full' : 'mobile')}>Open</a></div>
      <div class="setting"><div class="lbl">Version</div><div class="val">{APP_VERSION}</div></div>
    </div>
  {:else if name === 'controller'}
    <div class="setting-group"><h3>Controller</h3>
      <div class="setting"><div class="lbl">Link<small>UDP to the motion card</small></div><div class="val">{connection.state === 'connected' ? connection.endpoint : connection.state === 'faulted' ? 'lost' : 'disconnected'}</div></div>
      <div class="setting"><div class="lbl">Identity<small>product · firmware</small></div><div class="val">{machine.identity ? `${machine.identity.product_id} · ${machine.identity.program_version}` : '—'}</div></div>
      <div class="setting"><div class="lbl">Scale · cycle<small>{unitLabel('units/mm')} · interpolation</small></div><div class="val">{feedback ? `${displayNumber(feedback.scale, 'units/mm')} · ${feedback.cycle_us} µs` : '—'}</div></div>
      <div class="setting"><div class="lbl">Machine backup<small>{doc.files.backup?.name ?? doc.bindings_error ?? 'None loaded'}</small></div><MachineFiles disabled={!!machine.operation} /></div>
    </div>
  {:else if name === 'laser'}
    <div class="setting-group"><h3>Laser and head</h3>
      <div class="setting"><div class="lbl">Operating mode<small>Switch it in Run → Machine tests</small></div><div class="val">{laserLabel(doc.mode)}</div></div>
      <div class="setting"><div class="lbl">Applied on this connection</div><div class="val">{machine.session.mode ? laserLabel(machine.session.mode) : 'not yet'}</div></div>
      <div class="setting"><div class="lbl">Head controller<small>From the machine backup</small></div><div class="val">{bindings ? (bindings.head_enabled ? 'configured' : 'none') : '—'}</div></div>
    </div>
  {:else if name === 'axes'}
    <div class="setting-group"><h3>Axes and travel</h3>
      <div class="setting"><div class="lbl">X travel<small>soft limits</small></div><div class="val">{bindings ? `${distance(bindings.extent[0][0])} to ${quantity(bindings.extent[0][1], 'mm')}` : '—'}</div></div>
      <div class="setting"><div class="lbl">Y travel<small>soft limits</small></div><div class="val">{bindings ? `${distance(bindings.extent[1][0])} to ${quantity(bindings.extent[1][1], 'mm')}` : '—'}</div></div>
      <div class="setting"><div class="lbl">Jog speeds<small>slow · fast</small></div><div class="val">{bindings ? `${displayNumber(bindings.jog_speed[0], 'mm/s')} · ${quantity(bindings.jog_speed[1], 'mm/s')}` : '—'}</div></div>
      <div class="setting"><div class="lbl">Home<small>on this connection</small></div><div class="val">{machine.session.homed ? 'done' : 'needed'}</div></div>
      <div class="setting"><div class="lbl">Axis parameters<small>Compared with the backup</small></div><div class="val">{machine.session.parameters_verified ? 'checked' : 'not read'}</div></div>
    </div>
  {:else if name === 'safety'}
    <div class="setting-group"><h3>Safety inputs</h3>
      {#each bindings?.rules ?? [] as rule}
        <div class="setting"><div class="lbl">{rule.label}<small>Input {rule.input}</small></div><div class="val">{feedback ? (inputOn(rule.input) ? 'high' : 'low') : '—'}</div></div>
      {:else}
        <div class="setting"><div class="lbl">No input rules<small>{doc.bindings_error ?? 'Load the machine backup and connect'}</small></div></div>
      {/each}
    </div>
  {:else if name === 'materials'}
    <div class="setting-group"><h3>Material library</h3>
      <div class="setting"><div class="lbl">Recipes<small>{doc.library.recipes.length} saved</small></div><button class="btn btn-ghost" onclick={() => (ui.tab = 'materials')}>Open library</button></div>
    </div>
  {:else if name === 'process'}
    <MachineFiles kind="soft" disabled={!!machine.operation} />
  {:else if name === 'gas'}
    <GasCosts />
  {:else if name === 'head-calibration'}
    <div class="setting-group"><h3>Head calibration</h3>
      <div class="setting"><div class="lbl">On this connection<small>Calibrate from Run</small></div><div class="val">{machine.session.calibration ?? 'not yet'}</div></div>
    </div>
  {:else if name === 'matrix'}
    <MatrixCalibration />
  {:else if name === 'network'}
    <div class="setting-group"><h3>Controller network</h3>
      <div class="setting"><div class="lbl">Route<small>Network adapter</small></div><div class="val">{link.adapter ? `${link.adapter.description || link.adapter.name} · ${link.host ?? '—'}` : 'found on connection'}</div></div>
      <div class="setting"><div class="lbl">Last attempt<small>{link.phase === 'failed' ? 'failed' : link.phase === 'idle' ? 'ended' : 'in progress'}</small></div><div class="val">{link.detail || (connection.state === 'connected' ? 'connected' : '—')}</div></div>
      {#if link.choices.length}
        <div class="setting"><div class="lbl">Machine adapter<small>Used from the next connection</small></div><div class="seg">{#each link.choices as choice}<button class:on={choice.name === link.remembered_adapter} onclick={() => route('adapter', choice.name, link.remembered_adapter)}>{choice.description || choice.name}</button>{/each}</div></div>
      {/if}
      <div class="setting"><div class="lbl">Controller address<small>ip:port · used from the next connection</small></div><button class="val" onclick={() => osk.text('Controller address', link.controller, (v) => route('controller', v, link.controller))}>{link.controller}</button></div>
      <div class="setting"><div class="lbl">This computer's address<small>ip/prefix · used from the next connection</small></div><button class="val" onclick={() => osk.text("This computer's address", link.computer, (v) => route('host', v, link.computer))}>{link.computer}</button></div>
    </div>
  {:else if name === 'phones'}
    <div class="setting-group"><h3>Phones and tablets</h3>
      <div class="setting"><div class="lbl">Local network<small>{access.info?.address.replace('http://', '')}</small></div><button class="btn btn-ghost" onclick={() => (access.manage = true)}>Open</button></div>
    </div>
  {:else if name === 'xml'}
    <MachineSettings search={ui.settingsQuery} />
  {/if}
{/snippet}

<section class="panel main settings-page" class:section-only={sectionOnly}>
  {#if !sectionOnly}
    <nav class="settings-sidebar" aria-label="Settings sections">
      <div class="settings-title"><h1>Settings</h1><span>v{APP_VERSION}</span></div>
      <label class="search settings-search"><i class="ic ic-search"></i><input type="search" aria-label="Search settings" placeholder="Search settings" bind:value={ui.settingsQuery} /></label>
      {#each SETTINGS_PAGES as item (item.id)}
        <button class="settings-link" class:on={!query && ui.machinePage === item.id} aria-current={!query && ui.machinePage === item.id ? 'page' : undefined} onclick={() => { ui.settingsQuery = ''; ui.machinePage = item.id; }}>{item.label}</button>
      {/each}
    </nav>
  {/if}
  <div class="settings-content" bind:this={content}>
    {#if query}
      {#if !sectionOnly}<h2 class="settings-heading">Results for “{ui.settingsQuery.trim()}”</h2>{/if}
      {#each SETTINGS_PAGES as item (item.id)}
        {#each item.groups as name (name)}
          <div class="search-block" data-keywords="{item.label} {item.keywords ?? ''}"><p class="search-section">{item.label}</p>{@render group(name)}</div>
        {/each}
      {/each}
      {#if matches === 0}<p class="muted">Nothing matches. Try a shorter word.</p>{/if}
    {:else}
      {#if !sectionOnly}<h2 class="settings-heading">{page.label}</h2>{/if}
      <div class="settings-groups" class:wide-page={page.wide}>
        {#each page.groups as name (name)}<div class="search-block">{@render group(name)}</div>{/each}
      </div>
    {/if}
  </div>
</section>

{#if ui.checklistEditor}<PreflightEditor scope={ui.checklistEditor} onclose={() => (ui.checklistEditor = null)} />{/if}

<style>
  .settings-page { grid-column: 1 / -1; display: grid; grid-template-columns: 250px minmax(0, 1fr); gap: 0; padding: 0; overflow: hidden; }
  .settings-page.section-only { grid-template-columns: minmax(0, 1fr); }
  .settings-sidebar { display: flex; flex-direction: column; gap: 4px; padding: 16px 12px; border-right: 1px solid var(--line); overflow-y: auto; }
  .settings-title { display: flex; align-items: baseline; gap: 8px; padding: 0 8px 8px; }
  .settings-title span { font-size: var(--t-sm); color: var(--ink-3); }
  .settings-search { flex: none; min-width: 0; margin: 0 0 8px; }
  .settings-link { min-height: 48px; padding: 0 12px; border: 0; border-radius: 10px; background: transparent; color: var(--ink-2); font: inherit; font-weight: 600; text-align: left; cursor: pointer; }
  .settings-link.on { background: var(--accent-soft); color: var(--accent-2); }
  .settings-content { min-width: 0; overflow-y: auto; padding: 16px 22px 22px; display: flex; flex-direction: column; gap: 12px; }
  .settings-heading { margin: 4px 0; }
  .settings-groups { display: grid; grid-template-columns: repeat(auto-fit, minmax(340px, 1fr)); gap: 12px; align-items: start; }
  .settings-groups.wide-page { grid-template-columns: minmax(0, 1fr); }
  .search-section { margin: 8px 0 6px; font-size: var(--t-sm); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: .06em; }
  @media (max-width: 900px) { .settings-page { grid-template-columns: 200px minmax(0, 1fr); } }
</style>
