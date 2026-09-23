<script lang="ts">
  import { onMount } from 'svelte';
  import { access, layout } from './lib/access.svelte';
  import { subscribe } from './api/client';
  import { server } from './stores/server.svelte';
  import { ui } from './stores/ui.svelte';
  import { pending } from './lib/pending.svelte';
  import ControlGate from './components/ControlGate.svelte';
  import Confirm from './components/Confirm.svelte';

  async function loadShell(which: 'mobile' | 'full') {
    if (which === 'mobile') return await import('./mobile/Mobile.svelte');
    return await import('./App.svelte');
  }
  // The shell follows the window; the pages' state lives in stores and survives the switch.
  const shell = $derived(loadShell(layout.current));
  $effect(() => { document.documentElement.dataset.layout = layout.current; });
  // Unsaved jobs change with the draft and the library; recount after each, on both shells.
  const draftRevision = $derived(server.doc?.draft_revision ?? 0);
  const libraryItems = $derived((server.doc?.library.parts.length ?? 0) + (server.doc?.library.jobs.length ?? 0));
  $effect(() => { void draftRevision; void libraryItems; void ui.modal; if (server.link) pending.refresh(); });
  onMount(() => {
    const stopAccess = access.mount();
    const stopLayout = layout.mount();
    const stopEvents = subscribe(server.apply, server.setLink);
    return () => { stopAccess(); stopLayout(); stopEvents(); };
  });
</script>

{#if !access.info}
  <div class="opening"><img src="/logo.svg" alt="OpenLaser" /><h1>{access.error ? 'Waiting for OpenLaser' : 'Connecting to your machine…'}</h1>{#if access.error}<p>{access.error}</p><button class="btn btn-primary" onclick={() => access.refresh()}>Try again</button>{/if}</div>
{:else}
  <div class="interface-shell" inert={!access.canControl}>{#await shell}<div class="opening">Opening OpenLaser…</div>{:then { default: Shell }}<Shell />{/await}</div>
{/if}
{#if access.manage}{#await import('./components/Network.svelte') then { default: Network }}<Network />{/await}{/if}
<ControlGate />
<Confirm />

<style>
  .interface-shell { display:contents; }
  .opening { min-height:100dvh; padding:24px; display:flex; align-items:center; justify-content:center; flex-direction:column; gap:24px; text-align:center; }
  .opening img { width:160px; }
  .opening h1 { font-size:var(--t-lg); }
  .opening p { max-width:450px; color:var(--ink-2); }
</style>
