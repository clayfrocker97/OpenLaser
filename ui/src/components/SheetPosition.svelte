<script lang="ts">
  import { distance, unitLabel } from '../lib/units.svelte';
  import { plain } from '../lib/plain';
  import { api } from '../api/client';
  import type { PlacementChange } from '../api';
  import { server } from '../stores/server.svelte';
  import { osk } from '../lib/osk.svelte';
  import { ui } from '../stores/ui.svelte';
  import { access } from '../lib/access.svelte';
  import HoldButton from './HoldButton.svelte';
  import { withBusy } from '../lib/busy';

  let { compact = false }: { compact?: boolean } = $props();

  let busy = $state(false);
  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft);
  const placement = $derived(draft?.placement);
  const fixed = $derived(placement?.mode === 'fixed');
  const completed = $derived(!!doc.execution && !doc.execution.frame && ['completed', 'stopped'].includes(doc.machine.program?.state ?? ''));
  const disabled = $derived(busy || !access.canControl || !!doc.machine.operation || !draft || completed || doc.machine.program?.state === 'held');
  const position = $derived(doc.machine.feedback?.position_mm.slice(0, 2) as [number, number] | undefined);

  // The method is chosen with a tap; only the held Set origin takes the position.
  let method = $state<'head' | 'fixed'>('head');
  $effect(() => { method = fixed ? 'fixed' : 'head'; });
  // Once the origin is set the box shrinks to one line until Change.
  let changing = $state(false);
  const collapsed = $derived(!!placement?.captured && !changing);

  function chooseEachRun(): void {
    method = 'head';
    if (fixed) void change({ kind: 'head' });
  }
  async function setOrigin(): Promise<void> {
    await change(method === 'fixed' ? { kind: 'fixed_head' } : { kind: 'set_origin' });
    changing = false;
  }

  async function change(value: PlacementChange): Promise<void> {
    if (busy) return;
    await withBusy((b) => (busy = b), () => api.placement(value));
  }

  function save(): void {
    const job = doc.library.jobs.find(j => j.id === draft?.job);
    const perform = (name: string): Promise<void> =>
      withBusy((b) => (busy = b), async () => { await api.saveJob(name); ui.say('Job saved'); });
    if (job) void perform(job.name);
    else osk.text('Job name', draft?.name || 'Job', name => { if (name.trim()) void perform(name.trim()); });
  }
</script>

{#if draft && placement}
  <section class="sheet-position" class:compact aria-label="Sheet origin">
    {#if collapsed}
      <div class="origin-line">
        <span><strong>Origin set</strong> · {fixed ? 'Absolute' : 'Each run'}{#if draft.origin} · X {distance(draft.origin[0])} · Y {distance(draft.origin[1])} {unitLabel('mm')}{/if}</span>
        {#if !placement.saved && (draft.job || fixed) && !completed}<button class="text-button" disabled={disabled || !draft.recipe} onclick={save}>Save job</button>{/if}
        {#if !completed}<button class="text-button" disabled={disabled} onclick={() => (changing = true)}>Change</button>{/if}
      </div>
    {:else}
      <div class="position-title"><strong>Sheet origin</strong>{#if changing}<button class="text-button" onclick={() => (changing = false)}>Done</button>{/if}</div>
      <div class="position-method" role="group" aria-label="Positioning method">
        <button aria-pressed={method === 'head'} disabled={disabled} title="Set the origin at the head for each new run" onclick={chooseEachRun}>Each run</button>
        <button aria-pressed={method === 'fixed'} disabled={disabled} title="Keep one machine position for a fixture" onclick={() => (method = 'fixed')}>Absolute</button>
      </div>
      {#if !completed}
        <HoldButton class="set-origin" kind="zero" disabled={disabled || !doc.readiness.set_origin.ok} title="Set the origin where the head is" onhold={setOrigin}>
          Set origin here{#if position} · X {distance(position[0])} · Y {distance(position[1])}{/if}
        </HoldButton>
        {#if !disabled && !doc.readiness.set_origin.ok && doc.readiness.set_origin.reason}<p class="gate-reason origin-reason">{plain(doc.readiness.set_origin.reason).text}</p>{/if}
      {/if}
    {/if}
  </section>
{/if}

<style>
  .sheet-position { flex-shrink:0; display:grid; gap:6px; padding:8px 10px; border:1px solid var(--line); border-radius:12px; background:var(--panel-2); }
  .position-title, .origin-line { display:flex; justify-content:space-between; gap:8px; align-items:center; font-size:var(--t-sm); }
  .origin-line span { min-width:0; color:var(--ink-2); font-variant-numeric:tabular-nums; }
  .origin-line strong { color:var(--accent); }
  .position-method { display:grid; grid-template-columns:1fr 1fr; gap:6px; }
  .sheet-position :global(button) { min-height:44px; border:1px solid var(--line); border-radius:9px; background:var(--panel); color:var(--ink); font:inherit; font-size:var(--t-sm); cursor:pointer; }
  .sheet-position :global(button[aria-pressed="true"]) { border-color:var(--accent); background:var(--accent-soft); color:var(--accent); }
  .sheet-position :global(button:disabled) { opacity:.4; cursor:default; }
  .sheet-position :global(.set-origin) { width:100%; min-height:48px; padding:0 12px; border-color:var(--accent); color:var(--accent); font-variant-numeric:tabular-nums; }
  .origin-reason { margin:0; font-size:var(--t-sm); }
  .sheet-position .text-button { flex:none; min-width:44px; padding:0 8px; color:var(--accent); border:0; background:transparent; font-size:var(--t-sm); }
</style>
