<script lang="ts">
  import { api } from '../api/client';
  import { access } from '../lib/access.svelte';
  import { connectionLabel } from '../lib/connection';
  import { WORKFLOW, SETTINGS, SETTINGS_PAGES } from '../lib/navigation';
  import { TOOLS } from '../lib/features';
  import { recipeEdits } from '../lib/recipe-edits.svelte';
  import { settingsEdits } from '../lib/settings-edits.svelte';
  import { server } from '../stores/server.svelte';
  import { ui, type FeatureId, type Tab } from '../stores/ui.svelte';
  import { explain } from '../lib/format';
  import WorkspaceOverlays from '../components/WorkspaceOverlays.svelte';
  import Parts from './Parts.svelte';
  import Setup from './Setup.svelte';
  import Run from './Run.svelte';
  import Settings from './Settings.svelte';
  import SetupEditor from '../routes/Setup.svelte';
  import RunReview from '../routes/Run.svelte';
  import RunSide from '../routes/RunSide.svelte';
  import Machine from '../routes/Machine.svelte';
  import Materials from '../routes/Materials.svelte';
  import './mobile.css';

  const doc = $derived(server.doc);
  const connected = $derived(doc?.machine.connection.state === 'connected');
  const linkBusy = $derived(!!doc && doc.link.phase !== 'idle' && doc.link.phase !== 'failed');
  const done = $derived({ parts: !!doc?.draft, setup: !!doc?.draft?.compiled, run: false });
  const persistenceError = $derived(doc?.persistence_error || recipeEdits.storageError || settingsEdits.error);
  const settingsName = (page: number): string => SETTINGS_PAGES.find(item => item.id === page)?.label ?? 'Settings';
  type Detail = { tab:Tab; kind:'setup' | 'layout' | 'controls' | 'recovery' | 'machine'; title:string; restart?:boolean };
  let detail = $state<Detail | null>(ui.tab === 'machine' ? { tab:'machine',kind:'machine',title:settingsName(ui.machinePage) } : null);
  const section = $derived(detail?.tab === ui.tab ? detail : null);
  let connecting = $state(false);
  $effect(() => { if (section?.kind === 'setup' && !ui.setupPanel) back(); });
  $effect(() => { if (ui.tab === 'machine' && ui.checklistEditor) setting(1); });

  function back(): void {
    if (section?.tab === 'setup') { ui.setupPanel = null; ui.picking = null; ui.nestPicking = false; ui.nestPreview = null; }
    detail = null;
    if (ui.tab === 'materials') ui.tab = 'machine';
  }
  function go(tab:Tab): void { back(); ui.tab = tab; }
  function edit(tool:FeatureId | 'copy' | 'nest' | 'layout'): void {
    ui.setupPanel = tool === 'layout' ? null : tool;
    detail = { tab:'setup',kind:tool === 'layout' ? 'layout' : 'setup',title:tool === 'layout' ? 'Edit layout' : tool === 'copy' ? 'Copy machining' : tool === 'nest' ? 'Nest parts' : TOOLS.find(t => t.id === tool)!.name };
  }
  function setting(page:number): void { ui.machinePage = page; detail = { tab:'machine',kind:'machine',title:settingsName(page) }; }
  async function connect(): Promise<void> {
    if (connecting) return;
    connecting = true;
    try { await api.machine(connected ? 'disconnect' : linkBusy ? 'cancel' : 'connect'); }
    catch (error) { ui.say(explain(error), true); }
    finally { connecting = false; }
  }
  async function stop(): Promise<void> {
    try { await api.machine('stop'); } catch (error) { ui.say(explain(error), true); }
  }
</script>

<div class="phone-app">
  <header class="phone-header">
    <img src={ui.theme === 'dark' ? '/logo-white.svg' : '/logo.svg'} alt="OpenLaser" />
    <div>
      <button class="phone-connection" class:connected disabled={!access.canControl || !server.link || connecting} onclick={connect} title={doc?.link.detail || undefined}><span class="connection-dot"></span>{connectionLabel(doc)}{#if linkBusy}<small>Cancel</small>{/if}</button>
      <button class="phone-icon phone-alarm" aria-label="Alarms" onclick={() => ui.modal = 'alarms'}><i class="ic ic-bell"></i>{#if doc?.machine.alarms.length}<b>{doc.machine.alarms.length}</b>{/if}</button>
    </div>
  </header>
  {#if persistenceError}<div class="phone-notice" role="alert">{persistenceError}</div>{/if}
  {#if doc && !server.link}<div class="phone-notice" role="status">Connection lost · reconnecting…</div>{/if}
  {#if section || ui.tab === 'materials'}
    <div class="phone-detail-header"><button class="phone-icon" aria-label={ui.tab === 'materials' || ui.tab === 'machine' ? 'Back to Settings' : ui.tab === 'run' ? 'Back to Run' : 'Back to Setup'} onclick={back}><i class="ic ic-arrow-left"></i></button><h1>{section?.title ?? 'Material library'}</h1></div>
  {/if}
  <main class="phone-content" class:control-page={!section && (ui.tab === 'setup' || ui.tab === 'run') || section?.kind === 'controls'} class:phone-detail-setup={section?.kind === 'setup' || section?.kind === 'layout'} class:phone-layout-editor={section?.kind === 'layout'} class:phone-detail-machine={section?.kind === 'machine'} class:phone-detail-recovery={section?.kind === 'recovery'} class:phone-detail-materials={ui.tab === 'materials'}>
    {#if !doc}<div class="phone-empty"><h2>Waiting for the server</h2></div>
    {:else if section?.kind === 'controls'}<div class="phone-jog"><RunSide /></div>
    {:else if section?.kind === 'setup' || section?.kind === 'layout'}<SetupEditor compact />
    {:else if section?.kind === 'recovery'}<RunReview compact recoveryEditor={section.restart ?? false} />
    {:else if section?.kind === 'machine'}<Machine compact sectionOnly />
    {:else if ui.tab === 'parts'}<Parts />
    {:else if ui.tab === 'setup'}<Setup {edit} />
    {:else if ui.tab === 'run'}<Run controls={() => detail = {tab:'run',kind:'controls',title:'Controls'}} review={(restart = false) => detail = {tab:'run',kind:'recovery',title:restart ? 'Restart editor' : 'Run preview',restart}} />
    {:else if ui.tab === 'materials'}<Materials compact />
    {:else}<Settings open={setting} materials={() => ui.tab = 'materials'} />{/if}
  </main>
  <div class="phone-stop"><button disabled={!doc?.readiness.stop.ok || !server.link} onclick={stop}><i class="ic ic-stop"></i>Stop motion</button></div>
  <nav class="phone-nav" aria-label="Main navigation">
    {#each WORKFLOW as step}<button aria-current={ui.tab === step.id ? 'page' : undefined} onclick={() => go(step.id)}><span class="phone-step" class:complete={done[step.id]}>{#if done[step.id]}<i class="ic ic-check"></i>{:else}{step.number}{/if}</span>{step.label}</button>{/each}
    <button aria-current={ui.tab === SETTINGS.id || ui.tab === 'materials' ? 'page' : undefined} onclick={() => go(SETTINGS.id)}><span class="phone-step"><i class="ic ic-gear"></i></span>{SETTINGS.label}</button>
  </nav>
</div>
<WorkspaceOverlays nativeKeyboard />
