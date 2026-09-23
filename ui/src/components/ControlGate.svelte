<script lang="ts">
  import { access } from '../lib/access.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { explain } from '../lib/format';

  let error = $state('');
  const moving = $derived(!!server.doc?.machine.operation);
  async function take(): Promise<void> {
    error = '';
    try { await access.act('control'); }
    catch (failure) { error = explain(failure); }
  }
  async function stop(): Promise<void> {
    error = '';
    try { await api.machine('stop'); }
    catch (failure) { error = explain(failure); }
  }
</script>

{#if access.info && !access.canControl}
  <div class="control-blocker">
    <div role="dialog" aria-modal="true" aria-labelledby="control-title" class="control-dialog">
      <div class="control-symbol"><i class="ic ic-target"></i></div>
      <h1 id="control-title">{access.error ? 'Reconnecting to OpenLaser' : access.info.owner ? `${access.info.owner} has control` : 'Use OpenLaser here'}</h1>
      <p>{access.error ? 'Your workspace will return when the connection is restored.' : moving ? 'Wait for motion to finish, or stop it here.' : 'Take control to use this screen.'}</p>
      {#if error}<p class="control-error" role="alert">{error}</p>{/if}
      {#if access.error}<button class="take-control" onclick={() => access.refresh()}>Try again</button>
      {:else}<button class="take-control" disabled={access.busy || moving} onclick={take}>{access.busy ? 'Switching…' : 'Take control'}</button>{/if}
      {#if server.doc?.readiness.stop.ok && server.link}<button class="stop-motion" onclick={stop}><i class="ic ic-stop"></i>Stop motion</button>{/if}
    </div>
  </div>
{/if}

<style>
  .control-blocker { position:fixed; inset:0; z-index:200; display:grid; place-items:center; overflow:auto; padding:24px; background:color-mix(in srgb,var(--bg),transparent 18%); backdrop-filter:blur(5px); }
  .control-dialog { width:min(100%,380px); display:grid; justify-items:center; gap:18px; padding:30px 24px 24px; border:1px solid var(--line); border-radius:24px; background:var(--panel); box-shadow:var(--shadow); text-align:center; }
  .control-symbol { display:grid; place-items:center; width:56px; height:56px; border:1px solid var(--line); border-radius:17px; color:var(--accent); background:var(--accent-soft); font-size:var(--t-xl); }
  h1 { font-size:var(--t-xl); line-height:1.2; letter-spacing:-.025em; overflow-wrap:anywhere; }p { margin:0; font-size:var(--t-base); color:var(--ink-2); line-height:1.5; }.control-error { color:var(--stop); }
  button { width:100%; min-height:56px; border-radius:13px; font-size:var(--t-base); font-weight:650; cursor:pointer; touch-action:manipulation; }button:disabled { opacity:.4; cursor:default; }
  .take-control { border:0; color:#fff; background:var(--accent); }.stop-motion { display:flex; justify-content:center; align-items:center; gap:9px; border:1px solid var(--stop); background:var(--stop-soft); color:var(--stop); }
  @media (max-height:500px) { .control-blocker { padding:12px; }.control-dialog { padding:18px; gap:10px; }.control-symbol { display:none; } }
</style>
