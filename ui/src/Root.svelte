<script lang="ts">
  import { onMount } from 'svelte';
  import { access, chooseLayout } from './lib/access.svelte';
  import { subscribe } from './api/client';
  import { server } from './stores/server.svelte';
  import ControlGate from './components/ControlGate.svelte';

  const layout = chooseLayout();
  async function loadShell() {
    if (layout === 'mobile') return await import('./mobile/Mobile.svelte');
    return await import('./App.svelte');
  }
  const shell = loadShell();
  onMount(() => {
    document.documentElement.dataset.layout = layout;
    const stopAccess = access.mount();
    const stopEvents = subscribe(server.apply, server.setLink);
    return () => { stopAccess(); stopEvents(); };
  });
</script>

{#if !access.info}
  <div class="opening"><img src="/logo.svg" alt="OpenLaser" /><h1>{access.error ? 'Waiting for OpenLaser' : 'Connecting to your machine…'}</h1>{#if access.error}<p>{access.error}</p><button class="btn btn-primary" onclick={() => access.refresh()}>Try again</button>{/if}</div>
{:else}
  <div class="interface-shell" inert={!access.canControl}>{#await shell}<div class="opening">Opening OpenLaser…</div>{:then { default: Shell }}<Shell />{/await}</div>
{/if}
{#if access.manage}{#await import('./components/Network.svelte') then { default: Network }}<Network />{/await}{/if}
<ControlGate />

<style>
  .interface-shell { display:contents; }
  .opening { min-height:100dvh; padding:24px; display:flex; align-items:center; justify-content:center; flex-direction:column; gap:24px; text-align:center; }.opening img { width:160px; }.opening h1 { font-size:20px; }.opening p { max-width:450px; color:var(--ink-2); }
</style>
