<script lang="ts">
  import { untrack } from 'svelte';
  import type { ExecutionView, PreflightReview } from '../api';
  import { api } from '../api/client';
  import { beginRun } from '../lib/run-actions';
  import { access } from '../lib/access.svelte';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain, recipeLabel, seconds } from '../lib/format';
  import { diagnosticText, quantity } from '../lib/units.svelte';
  import Preview from './Preview.svelte';
  import FlightChecklist from '../components/FlightChecklist.svelte';
  import RunControls from '../components/RunControls.svelte';
  import HoldButton from '../components/HoldButton.svelte';
  import { onlyPart } from '../lib/job-parts';

  let { controls, review }: { controls:() => void; review:(restart?: boolean) => void } = $props();
  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft);
  const part = $derived(onlyPart(draft, doc.library.parts));
  const execution = $derived(doc.execution);
  let original = $state<ExecutionView | null>(null);
  $effect(() => {
    const id = doc.recovery?.id;
    if (id && original?.id !== id) untrack(() => {
      if (execution?.id === id) original = execution;
      else api.recoveryProgram().then(reply => { if (server.doc?.recovery?.id === reply.execution.id) original = reply.execution; }).catch(error => ui.say(explain(error), true));
    });
  });
  const retained = $derived(execution && !execution.frame && original?.id === doc.recovery?.id ? original : null);
  const compiled = $derived(retained?.compiled ?? execution?.compiled ?? draft?.compiled);
  const material = $derived(execution?.material ?? draft?.recipe);
  const name = $derived(execution?.name ?? (draft?.name || 'Current job'));
  const program = $derived(doc.machine.program);
  const running = $derived(program?.state === 'running' || program?.state === 'finishing');
  const paused = $derived(program?.state === 'held');
  const completed = $derived(!!execution && !execution.frame && program?.state === 'completed');
  const recovering = $derived(!!doc.recovery && ['held','stopped','failed'].includes(doc.recovery.state));
  const total = $derived(compiled?.plan.length ?? 0);
  const done = $derived(retained ? doc.recovery?.steps.filter(step => step.status === 'completed').length ?? 0 : execution ? doc.progress?.completed ?? 0 : 0);
  const percent = $derived(total ? Math.min(100,Math.round(done / total * 100)) : 0);
  const outline = $derived(execution ? compiled!.moves.filter(m => m.kind === 'cut').map(m => m.points) : draft?.preview?.contours.flatMap(c => c.paths.filter(p => p.kind === 'cut').map(p => p.points)) ?? part?.outline ?? []);
  let busy = $state(false);
  let preflight = $state<PreflightReview | null>(null);
  const blocked = $derived(!access.canControl || busy || !server.link);
  async function perform(action:() => Promise<unknown>): Promise<void> {
    if (busy) return;
    busy = true;
    try { await action(); } catch (error) { ui.say(explain(error),true); }
    finally { busy = false; }
  }
  function act(action:'run' | 'resume' | 'hold' | 'stop'): void {
    void perform(async () => {
      if (action === 'run' || action === 'resume') preflight = await beginRun(action, doc);
      else await api.machine(action);
    });
  }
</script>

<div class="phone-run">
  <div class="phone-run-scroll">
    <div class="phone-page-title"><h1>Run</h1>{#if compiled}<span class="phone-mode">{completed ? 'Finished' : running ? execution?.frame ? 'Frame' : compiled.dry_run ? 'Dry run' : 'Running' : paused ? 'Paused' : program?.state === 'stopped' ? 'Stopped' : program?.state === 'failed' ? 'Interrupted' : compiled.dry_run ? 'Dry run' : 'Cut'}</span>{/if}</div>
    {#if draft}
      {#if !running && !recovering}
        <div class="seg phone-run-choice" role="group" aria-label="Run type">
          <button class:on={!draft.dry_run} disabled={blocked || !doc.readiness.compile.ok || !!doc.machine.operation} onclick={() => perform(() => api.compile(false))}>Cut</button>
          <button class:on={draft.dry_run} disabled={blocked || !doc.readiness.compile.ok || !!doc.machine.operation} onclick={() => perform(() => api.compile(true))}>Dry run</button>
        </div>
      {/if}
      <section class="phone-job-card">
        <div class="phone-job-heading"><div><h2>{name}</h2><p class="phone-job-material">{material ? recipeLabel(material) : 'No material'}</p></div></div>
        <button class="phone-preview-button" aria-label="View toolpath" onclick={() => review()}><Preview {outline} /></button>
        {#if compiled}<div class="phone-progress"><div><strong>{percent}<small>%</small></strong><span>{done} / {total} passes</span><span>{seconds(compiled.seconds * (1 - percent / 100))}<small>{running ? 'estimated left' : 'estimated time'}</small></span></div><progress aria-label="Job progress" max="100" value={percent}></progress></div>{/if}
      </section>
      {#if paused && doc.recovery?.pause_position}<p class="phone-pause-position" role="status">Paused at X {quantity(doc.recovery.pause_position[0], 'mm')} · Y {quantity(doc.recovery.pause_position[1], 'mm')}. Use Controls to move the head. Resume returns here.</p>{/if}
      {#if recovering}<button class="phone-review-restart" onclick={() => review(true)}>Adjust restart…<i class="ic ic-arrow-right"></i></button>{/if}
    {:else}<div class="phone-empty"><i class="ic ic-folder"></i><h2>No part open</h2><p>Choose a part or saved job to begin.</p><button class="phone-primary" onclick={() => ui.tab = 'parts'}>Go to Parts</button></div>{/if}
  </div>
  <div class="phone-run-footer">
    {#if draft}
      <RunControls onaction={act} {busy} showStop={false} explain={false} />
      {#if !draft.recipe && !recovering}<button class="phone-text-action phone-setup-link" onclick={() => ui.tab = 'setup'}>Choose material</button>
      {:else if !running && !completed && !recovering}<p class="phone-readiness">{diagnosticText(draft.error ?? doc.readiness.run.reason ?? '')}</p>{/if}
    {/if}
    <div class="phone-run-tools"><button onclick={controls}><i class="ic ic-target"></i>Controls</button><HoldButton class="" disabled={blocked || !doc.readiness.frame.ok} onhold={() => perform(() => api.machine('frame'))}><i class="ic ic-frame"></i>Frame</HoldButton></div>
  </div>
</div>
{#if preflight}<FlightChecklist initial={preflight} onclose={() => preflight = null} />{/if}

<style>.phone-run-choice { display:flex; margin-bottom:12px; }.phone-run-choice button { flex:1; min-height:44px; }.phone-pause-position { padding:12px; border:1px solid var(--hold); border-radius:9px; font-size:13px; line-height:1.5; }</style>
