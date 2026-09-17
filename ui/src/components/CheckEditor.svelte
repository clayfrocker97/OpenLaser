<script lang="ts">
  import { osk } from '../lib/osk.svelte';
  import { distance, unitLabel } from '../lib/units.svelte';
  import type { Check } from '../api';
  import { ACTIONS, BED_POINTS, canAutoCheck, movesMachine } from '../lib/preflight';
  let { steps = $bindable<Check[]>([]), phase = 'preflight' }: { steps: Check[]; phase?: 'preflight' | 'pause' | 'postflight' } = $props();
  const actions = $derived(phase === 'postflight' ? ACTIONS.filter(([kind]) => ['none', 'move_to', 'move_xy'].includes(kind)) : phase === 'pause' ? ACTIONS.filter(([kind]) => ['none', 'job_gas_test'].includes(kind)) : ACTIONS);
  function setAction(step: Check, kind: string): void {
    switch (kind) {
      case 'home': case 'origin': case 'calibrate': step.action = { kind }; break;
      case 'move_to': step.action = { kind, point: 'front_left' }; break;
      case 'job_gas_test': step.action = { kind, duration_ms: 500 }; break;
      case 'move_xy': step.action = { kind, x: 0, y: 0 }; break;
      default: step.action = null;
    }
    if (!canAutoCheck(step.action)) step.auto_check = false;
  }
</script>

<div class="check-editor">
  {#each steps as step, i}
    <div class="check-edit" class:motion={movesMachine(step.action)}>
      <div class="check-text">
        <span class="muted">{i + 1}</span>
        <textarea aria-label="Check {i + 1}" rows="2" maxlength="500" bind:value={step.text}></textarea>
        <button class="btn btn-ghost icon-only" aria-label="Remove check {i + 1}" onclick={() => (steps = steps.filter((_, index) => index !== i))}><i class="ic ic-x"></i></button>
      </div>
      <details>
        <summary>{step.action ? ACTIONS.find(([kind]) => kind === (step.action?.kind === 'gas_test' ? 'job_gas_test' : step.action?.kind))?.[1] : 'Optional action'}{#if movesMachine(step.action)}<span class="motion-label">Moves machine</span>{/if}</summary>
        <div class="action-fields">
          <fieldset class="choices"><legend>Action</legend><div class="choice-grid">{#each actions as [kind, title]}<button class="choice" aria-pressed={(step.action?.kind === 'gas_test' ? 'job_gas_test' : step.action?.kind ?? 'none') === kind} onclick={() => setAction(step, kind)}>{title}</button>{/each}</div></fieldset>
          {#if step.action?.kind === 'move_to'}
            {@const action = step.action}
            <fieldset class="choices"><legend>Bed point</legend><div class="choice-grid bed-points">{#each [...BED_POINTS.slice(6), ...BED_POINTS.slice(3, 6), ...BED_POINTS.slice(0, 3)] as [point, title]}<button class="choice" aria-pressed={action.point === point} onclick={() => (action.point = point)}>{title}</button>{/each}</div></fieldset>
          {/if}
          {#if step.action?.kind === 'move_xy'}
            {@const action = step.action}
            {#each ['x', 'y'] as axis}
              <label>{axis.toUpperCase()} · {unitLabel('mm')}<button class="coordinate" onclick={() => osk.number(`${axis.toUpperCase()} position`, action[axis as 'x' | 'y'], 'mm', value => { if (Number.isFinite(value)) action[axis as 'x' | 'y'] = value; })}>{distance(action[axis as 'x' | 'y'])}</button></label>
            {/each}
          {/if}
          {#if step.action?.kind === 'job_gas_test' || step.action?.kind === 'gas_test'}
            <p class="gas-note">Uses the compiled job’s gas.</p>
            <label>Duration · ms<input type="number" min="50" max="2000" step="50" bind:value={step.action.duration_ms}></label>
          {/if}
        </div>
        {#if canAutoCheck(step.action)}
          <label class="auto-check"><input type="checkbox" bind:checked={step.auto_check}>Auto-check from machine state</label>
        {:else if step.action?.kind === 'gas_test' || step.action?.kind === 'job_gas_test'}<p class="muted">Confirm flow after testing.</p>{/if}
      </details>
    </div>
  {/each}
  <button class="btn btn-ghost" disabled={steps.length >= 50} onclick={() => (steps = [...steps, { text: '', action: null, auto_check: false }])}><i class="ic ic-plus"></i>Add check</button>
</div>

<style>
  .check-editor { display: grid; gap: 10px; }
  .check-edit { padding: 10px; background: var(--panel-2); border: 1px solid var(--line); border-radius: 12px; }
  .check-edit.motion { border-color: var(--move-line); background: var(--move-soft);  }
  .motion-label { color: var(--move); margin-left: 12px; font-weight: 600; }
  .auto-check { display: flex; align-items: center; gap: 10px; margin-top: 14px; cursor: pointer; }
  .auto-check input { width: 22px; height: 22px; accent-color: var(--start); }
  .check-text { display: grid; grid-template-columns: 20px 1fr 44px; align-items: start; gap: 8px; }
  .check-text > span { padding-top: 12px; font-size: var(--t-sm); }
  .coordinate { min-height: 44px; text-align: left; cursor: pointer; }
  textarea, input, .coordinate { font: inherit; width: 100%; min-width: 0; border: 1px solid var(--line-2); background: var(--panel); color: var(--ink); border-radius: 8px; padding: 10px; }
  textarea { resize: vertical; line-height: 1.4; }
  details { margin: 8px 0 0 28px; }
  summary { cursor: pointer; color: var(--ink-2); font-size: var(--t-sm); padding: 5px 0; }
  .action-fields { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; padding-top: 8px; }
  .choices { grid-column: 1 / -1; padding: 0; margin: 0; border: 0; min-width: 0; }
  .choices legend { padding: 0 0 7px; font-size: var(--t-sm); color: var(--ink-2); }
  .choice-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px; }
  .bed-points { grid-template-columns: repeat(3, minmax(0, 1fr)); }
  .choice { min-height: 44px; padding: 9px 6px; border: 1px solid var(--line); border-radius: 8px; background: var(--panel); color: var(--ink-2); font: inherit; font-size: var(--t-sm); cursor: pointer; }
  .choice[aria-pressed="true"] { border-color: var(--accent); color: var(--accent); background: var(--accent-soft); }
  label { display: grid; gap: 5px; color: var(--ink-2); font-size: var(--t-sm); }
  .gas-note { align-self: end; }
  p { font-size: var(--t-sm); margin: 8px 0 0; }
</style>
