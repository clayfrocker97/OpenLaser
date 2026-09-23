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
  const position = $derived(draft?.origin ?? (doc.machine.feedback?.position_mm.slice(0, 2) as [number, number] | undefined));

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
  <section class="sheet-position" class:compact aria-label="Sheet position">
    <div class="position-title"><strong>Sheet position</strong><span>{draft.anchor === 'front_left' ? 'Bottom left' : draft.anchor.replaceAll('_', ' ')}</span>{#if !placement.saved && (draft.job || fixed) && !completed}<button class="text-button save-position" disabled={disabled || !draft.recipe} onclick={save}>Save job</button>{/if}</div>
    <div class="position-method" role="group" aria-label="Positioning method">
      <button aria-pressed={!fixed} disabled={disabled} title="Set the origin at the head for each new run" onclick={() => change({ kind: 'head' })}>Each run</button>
      <HoldButton class="position-absolute" kind="zero" aria-pressed={fixed} disabled={disabled || !doc.readiness.set_origin.ok} title="Save the head's absolute position for a fixture" onhold={() => change({ kind: 'fixed_head' })}>Absolute</HoldButton>
    </div>
    <div class="origin-state" class:captured={placement.captured}>{placement.captured ? 'Origin set' : 'Origin not set'}{#if !placement.captured && !fixed}<small>Set at head for this job</small>{/if}</div>
    {#if position}<div class="position-state"><strong>{placement.captured ? '' : 'Head · '}X {distance(position[0])} · Y {distance(position[1])} {unitLabel('mm')}</strong></div>{/if}
    {#if !completed}<HoldButton class="set-origin" kind="zero" disabled={disabled || !doc.readiness.set_origin.ok} title={doc.readiness.set_origin.reason ?? 'Set origin at the head'} onhold={() => change({ kind: 'set_origin' })}>Set origin</HoldButton>
    {#if !disabled && !doc.readiness.set_origin.ok && doc.readiness.set_origin.reason}<p class="gate-reason origin-reason">{plain(doc.readiness.set_origin.reason).text}</p>{/if}{/if}
  </section>
{/if}

<style>
  .sheet-position { flex-shrink:0; padding:8px 10px; border:1px solid var(--line); border-radius:12px; background:var(--panel-2); }
  .position-title { display:flex; justify-content:space-between; gap:8px; align-items:center; min-height:28px; margin-bottom:5px; font-size:var(--t-sm); }
  .position-title span { color:var(--ink-3); font-size:var(--t-sm); text-transform:capitalize; }
  .position-method { display:grid; grid-template-columns:1fr 1fr; gap:6px; }
  .sheet-position :global(button) { min-height:44px; border:1px solid var(--line); border-radius:9px; background:var(--panel); color:var(--ink); font:inherit; font-size:var(--t-sm); cursor:pointer; }
  .sheet-position :global(button[aria-pressed="true"]) { border-color:var(--accent); background:var(--accent-soft); color:var(--accent); }
  .sheet-position :global(button:disabled) { opacity:.4; cursor:default; }
  .position-state { display:flex; align-items:center; padding-top:5px; min-height:32px; }
  .origin-state { display:flex; align-items:center; justify-content:space-between; gap:8px; margin-top:8px; color:var(--ink-3); font-size:var(--t-sm); }
  .origin-state.captured { color:var(--accent); }
  .origin-state small { font-size:var(--t-sm); }
  .sheet-position :global(.set-origin) { width:100%; min-height:48px; margin-top:6px; padding:0 12px; border-color:var(--accent); color:var(--accent); }
  .origin-reason { grid-column:1 / -1; font-size:var(--t-sm); }
  .position-state strong { font-size:var(--t-sm); font-weight:550; font-variant-numeric:tabular-nums; }
  .sheet-position .text-button { padding:0 6px; color:var(--accent); border:0; background:transparent; font-size:var(--t-sm); }
  .save-position { margin-block:-8px; }
  .compact { display:grid; grid-template-columns:minmax(0,2fr) minmax(0,1fr); gap:6px; }
  .compact .position-title { grid-column:1 / -1; margin:0; min-height:22px; }
  .compact .origin-state { grid-row:2; grid-column:1 / -1; margin:0; font-size:var(--t-sm); }
  .compact .origin-state small { display:none; }
  .compact .position-state { grid-row:2; grid-column:1 / -1; justify-content:flex-end; padding:0; min-height:26px; }
  .compact .position-state strong { font-size:var(--t-sm); }
  .compact .position-method { grid-row:3; }
  .compact .position-method :global(button) { min-height:46px; font-size:var(--t-sm); }
  .compact :global(.set-origin) { grid-row:3; margin:0; min-height:46px; padding:0 6px; font-size:var(--t-sm); }
</style>
