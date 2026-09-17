<script lang="ts">
  import Brand from './components/Brand.svelte';
  import WorkspaceOverlays from './components/WorkspaceOverlays.svelte';
  import { recipeEdits } from './lib/recipe-edits.svelte';
  import { settingsEdits } from './lib/settings-edits.svelte';
  import Parts from './routes/Parts.svelte';
  import Materials from './routes/Materials.svelte';
  import Setup from './routes/Setup.svelte';
  import Run from './routes/Run.svelte';
  import Machine from './routes/Machine.svelte';
  import { api } from './api/client';
  import { server } from './stores/server.svelte';
  import { ui, type Tab } from './stores/ui.svelte';
  import { explain } from './lib/format';
  import { WORKFLOW, SETTINGS } from './lib/navigation';
  import { connectionLabel } from './lib/connection';

  const doc = $derived(server.doc);
  const connected = $derived(doc?.machine.connection.state === 'connected');
  // The connect workflow's step, on the button; a click while it runs cancels.
  const link = $derived(doc?.link);
  const busy = $derived(!!link && link.phase !== 'idle' && link.phase !== 'failed');
  const label = $derived(connectionLabel(doc));
  const alarms = $derived(doc?.machine.alarms.length ?? 0);
  const brandSub = $derived.by(() => {
    if (!doc?.draft) return '';
    const part = doc.library.parts.find((p) => p.id === doc.draft?.part);
    return part ? part.file_name : '';
  });

  function setTab(tab: Tab): void {
    ui.tab = tab;
  }

  function togglePage(page: Tab): void {
    ui.tab = ui.tab === page ? 'parts' : page;
  }

  async function connect(): Promise<void> {
    try {
      await api.machine(connected ? 'disconnect' : busy ? 'cancel' : 'connect');
    } catch (error) {
      ui.say(explain(error), true);
    }
  }

  const done = $derived({
    parts: !!doc?.draft,
    setup: !!doc?.draft?.compiled,
  });
</script>

<div class="app">
  <header class="topbar">
    <div class="brand">
      <Brand />
      <span class="brand-sub">{brandSub}</span>
    </div>

    <nav class="tabs">
      {#each WORKFLOW as { id: tab, number: num, label }}
        <button class="tab" class:active={ui.tab === tab} class:done={done[tab as 'parts' | 'setup'] ?? false} onclick={() => setTab(tab as Tab)}>
          <span class="tab-num">{#if done[tab as 'parts' | 'setup']}<i class="ic ic-check"></i>{:else}{num}{/if}</span><span>{label}</span>
        </button>
      {/each}
    </nav>

    <div class="topbar-actions">
      <button class="btn btn-ghost page-btn" onclick={() => (ui.modal = 'pending')}>Pending changes</button>
      <button class="btn btn-ghost icon-btn page-btn" class:active={ui.tab === 'materials'} onclick={() => togglePage('materials')} title="Material library"><i class="ic ic-layers"></i><span>Materials</span></button>
      <button class="btn btn-ghost icon-btn page-btn" class:active={ui.tab === SETTINGS.id} onclick={() => togglePage(SETTINGS.id)} title={SETTINGS.label}><i class="ic ic-gear"></i><span>{SETTINGS.label}</span></button>
      <button class="btn btn-ghost icon-btn alarm" onclick={() => (ui.modal = 'alarms')} title="Alarms"><i class="ic ic-bell"></i><span>Alarms</span>{#if alarms > 0}<span class="badge">{alarms}</span>{/if}</button>
      <button class="btn btn-primary connect" class:connected class:busy onclick={connect} disabled={!server.link} title={link?.detail ?? ''}><span class="dot"></span><span>{label}</span>{#if busy}<span class="cancel">Cancel</span>{/if}</button>
    </div>
  </header>

  {#if doc?.persistence_error || recipeEdits.storageError || settingsEdits.error}<div class="persistence-error" role="alert">{doc?.persistence_error ?? recipeEdits.storageError ?? settingsEdits.error}</div>{/if}

  <main class="workspace" class:materials={ui.tab === 'materials'}>
    {#if !doc}
      <section class="panel main"><div class="empty"><h2>Waiting for the server</h2>Start <span class="mono">openlaser</span> and this page connects on its own.</div></section>
    {:else if ui.tab === 'parts'}
      <Parts />
    {:else if ui.tab === 'materials'}
      <Materials />
    {:else if ui.tab === 'setup'}
      <Setup />
    {:else if ui.tab === 'run'}
      <Run />
    {:else}
      <Machine />
    {/if}
  </main>

  <WorkspaceOverlays />
</div>

<style>.app:has(.persistence-error) { grid-template-rows:68px auto minmax(0,1fr); }.persistence-error { padding: 8px 20px; background: var(--warn-soft); color: var(--warn); }</style>
