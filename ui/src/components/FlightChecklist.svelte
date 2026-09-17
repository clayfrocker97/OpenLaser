<script lang="ts">
  import { diagnosticText } from '../lib/units.svelte';
  import { untrack } from 'svelte';
  import Modal from './Modal.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain } from '../lib/format';
  import type { PreflightReview, PostflightReview } from '../api';
  import { actionName, movesMachine } from '../lib/preflight';

  let { initial, onclose }: { initial: PreflightReview | PostflightReview; onclose: () => void } = $props();
  let review = $state<PreflightReview | PostflightReview>(untrack(() => initial));
  let checked = $state<number[]>([]);
  let gasReady = $state(false);
  let submitting = $state(false);
  let acting = $state(false);
  let loading = $state(false);
  let polling = false;
  let refreshId = 0;
  let error = $state('');
  const doc = $derived(server.doc!);
  const preflight = $derived('intent' in review ? review : null);
  const postflight = $derived('notice' in review ? review : null);
  const token = $derived(preflight?.token ?? postflight!.notice.id);
  const readiness = $derived(preflight?.intent === 'run' ? doc.readiness.run : preflight ? doc.readiness.resume : { ok: true, reason: null });
  const busy = $derived(acting || !!doc.machine.operation);
  const total = $derived(review.steps.length + Number(!!preflight?.confirm_gas));
  const satisfied = $derived(busy ? [] : review.satisfied);
  const allChecked = $derived([...new Set([...checked, ...satisfied])].sort((a, b) => a - b));
  const done = $derived(allChecked.length + Number(!!preflight?.confirm_gas && gasReady));
  const complete = $derived(done === total);
  const title = $derived(postflight ? 'Postflight Checklist' : preflight?.intent === 'resume' ? 'Pause Checklist' : 'Preflight Checklist');
  const button = $derived(postflight ? 'Done' : preflight?.intent === 'resume' ? 'Resume' : preflight?.dry_run ? 'Start dry run' : 'Start cutting');

  function toggle(index: number, on: boolean): void {
    checked = on ? [...checked.filter((i) => i !== index), index] : checked.filter((i) => i !== index);
  }

  async function refresh(background = false): Promise<void> {
    const id = ++refreshId;
    if (!background) { loading = true; checked = []; gasReady = false; }
    try {
      const next = preflight ? await api.preflight(preflight.intent) : await api.postflight();
      if (!next) { onclose(); return; }
      if (id === refreshId) {
        if (token !== ('token' in next ? next.token : next.notice.id)) { checked = []; gasReady = false; }
        review = next;
      }
    }
    catch (e) { if (id === refreshId) { error = explain(e); review = { ...review, satisfied: [] }; checked = []; gasReady = false; } }
    finally { if (id === refreshId) loading = false; }
  }

  // A new draft, defaults edit, disconnect or alarm transition invalidates
  // local checks immediately. The server also checks the exact token at Start.
  const context = $derived(`${preflight ? doc.draft_revision : doc.postflight?.id}/${preflight ? doc.preflight_revision : ''}/${doc.machine.alarm_revision}/${preflight ? doc.recovery?.revision : ''}/${doc.mode}/${server.link}`);
  let previous = '';
  $effect(() => {
    const next = context;
    if (previous && previous !== next) {
      untrack(() => { error = 'Setup changed. Check again.'; void refresh(); });
    }
    previous = next;
  });

  // Automatic checks are current observations, not saved completion receipts.
  $effect(() => {
    const timer = setInterval(() => {
      if (polling || loading || busy || submitting || !server.link || !review.steps.some(step => step.auto_check)) return;
      polling = true;
      void refresh(true).finally(() => { polling = false; });
    }, 500);
    return () => { clearInterval(timer); refreshId++; };
  });

  let wasBusy = false;
  $effect(() => {
    const next = busy;
    // A gas test only refreshes observations. Keep manual checks unless the
    // review token says the job, setup or checklist actually changed.
    if (wasBusy && !next) untrack(() => { void refresh(true); });
    wasBusy = next;
  });

  async function action(index: number): Promise<void> {
    acting = true;
    error = '';
    try { if (postflight) await api.postflightAction(token, index); else await api.preflightAction(token, index); }
    catch (e) { error = explain(e); await refresh(); }
    finally { acting = false; }
  }

  async function start(): Promise<void> {
    if (!complete || submitting || busy || loading || !server.link) return;
    submitting = true;
    error = '';
    try {
      if (preflight) await api.machine(preflight.intent, { preflight: { token, checked: allChecked, gas_ready: gasReady } });
      else await api.dismissPostflight(token);
      onclose();
    } catch (e) { error = explain(e); await refresh(); }
    finally { submitting = false; }
  }

  function close(): void {
    if (postflight) void api.dismissPostflight(token).catch(e => ui.say(explain(e), true));
    onclose();
  }

  function edit(): void {
    ui.editChecklist(postflight ? 'postflight' : preflight?.intent === 'resume' ? 'pause' : 'defaults');
    onclose();
  }

  async function stop(): Promise<void> {
    try { await api.machine('stop'); } catch (e) { error = explain(e); }
  }
</script>

<Modal {title} onclose={() => { if (!submitting) close(); }}>
  {#snippet actions()}<button class="btn btn-ghost" aria-label={`Edit ${title.toLowerCase()}`} disabled={submitting || busy || loading} onclick={edit}>Edit</button>{/snippet}
  <div class="preflight-progress"><span>{postflight ? postflight.notice.name : preflight?.intent === 'resume' ? 'Before resuming' : preflight?.dry_run ? 'Dry run · laser off' : 'Machine and material'}</span><strong>{done} / {total}</strong></div>
  <div class="preflight-checks">
    {#each review.steps as step, i}
      <div class="preflight-check" class:checked={allChecked.includes(i)} class:motion={movesMachine(step.action)}>
        <label><input type="checkbox" checked={allChecked.includes(i)} disabled={submitting || busy || loading || satisfied.includes(i)} onchange={(event) => toggle(i, event.currentTarget.checked)}><span>{step.text}{#if movesMachine(step.action)}<small class="motion-label">Moves machine</small>{/if}{#if satisfied.includes(i)}<small>Auto-checked</small>{/if}</span></label>
        {#if step.action}<button class="btn" class:btn-move={movesMachine(step.action)} class:btn-ghost={!movesMachine(step.action)} disabled={busy || loading || submitting || !server.link} onclick={() => action(i)} title={movesMachine(step.action) ? 'Moves the machine' : step.action.kind === 'gas_test' ? 'Opens the gas briefly, then closes it' : 'Changes the job origin'}>{actionName(step.action)}</button>{/if}
      </div>
    {/each}
    {#if preflight?.confirm_gas}
      <div class="preflight-check" class:checked={gasReady}><label><input type="checkbox" bind:checked={gasReady} disabled={submitting || busy || loading}><span>Gas supply is ready<small>{preflight.gases.join(' · ')}</small></span></label></div>
    {/if}
  </div>
  {#if error}<p class="warn-text" role="alert">{error}</p>{:else if !readiness.ok && !submitting && readiness.reason !== 'a manual output is active'}<p class="muted">{diagnosticText(readiness.reason ?? '')}</p>{/if}
  <div class="preflight-footer">
    {#if busy}<button class="btn btn-stop" onclick={stop}>Stop motion</button>{:else}<button class="btn btn-ghost" onclick={close} disabled={submitting}>{postflight || preflight?.intent === 'resume' ? 'Close' : 'Cancel'}</button>{/if}
    <button class="btn lg" class:btn-start={!!preflight} class:btn-primary={!!postflight} disabled={!complete || !readiness.ok || !server.link || submitting || busy || loading} onclick={start}>{submitting ? postflight ? 'Closing…' : 'Starting…' : button}</button>
  </div>
</Modal>

<style>
  .preflight-progress { display: flex; align-items: center; justify-content: space-between; gap: 16px; color: var(--ink-2); font-size: var(--t-sm); }
  .preflight-progress strong { font-variant-numeric: tabular-nums; color: var(--ink); }
  .preflight-checks { display: grid; gap: 8px; padding: 4px 0; }
  .preflight-check { display: flex; align-items: center; gap: 10px; border: 1px solid var(--line); background: var(--panel-2); border-radius: 12px; padding: 0 12px; }
  .preflight-check.checked { border-color: var(--line-2); }
  .preflight-check.motion { border-color: var(--move-line); background: var(--move-soft);  }
  .motion-label { color: var(--move); font-weight: 600; }
  label { display: flex; align-items: center; gap: 13px; flex: 1; padding: 16px 0; cursor: pointer; }
  input { width: 22px; height: 22px; flex: 0 0 22px; accent-color: var(--start); }
  small { display: block; color: var(--ink-2); font-size: var(--t-sm); margin-top: 4px; }
  .preflight-footer { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding-top: 14px; border-top: 1px solid var(--line); }
  p { margin: 0; font-size: var(--t-sm); }
</style>
