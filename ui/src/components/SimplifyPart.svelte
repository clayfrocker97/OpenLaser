<script lang="ts">
  // Simplifies a part's drawing into a new part: runs of short lines become
  // the arcs and lines they follow within a tolerance, repeated contours and
  // specks go. The part itself stays as it is for the jobs that use it.
  import Modal from './Modal.svelte';
  import { api } from '../api/client';
  import type { PartView, SimplifyView } from '../api';
  import { ui } from '../stores/ui.svelte';
  import { explain } from '../lib/format';
  import { quantity } from '../lib/units.svelte';

  let { part, onclose }: { part: PartView; onclose: () => void } = $props();
  const TOLERANCES = [['Fine', 0.01], ['Standard', 0.02], ['Coarse', 0.05], ['Rough', 0.1]] as const;
  let tolerance = $state(0.02);
  let result = $state<SimplifyView | null>(null);
  let checking = $state(false), saving = $state(false), error = $state('');
  let asked = 0;

  const count = (n: number) => n.toLocaleString();
  const changed = $derived(!!result && (result.curves_after !== result.curves_before || result.repeats > 0 || result.specks > 0));

  // Every choice shows what it would do before anything is saved.
  $effect(() => {
    const at = tolerance, id = part.id, ask = ++asked;
    checking = true; error = '';
    api.simplifyPart(id, at, false)
      .then((view) => { if (ask === asked) result = view; })
      .catch((e) => { if (ask === asked) { result = null; error = explain(e); } })
      .finally(() => { if (ask === asked) checking = false; });
  });

  async function save(): Promise<void> {
    if (saving || !changed) return;
    // Selecting the new part replaces `part`, so nothing reads it afterwards.
    const { id, name } = part;
    saving = true;
    try {
      const view = await api.simplifyPart(id, tolerance, true);
      onclose();
      if (view.part) ui.selected = view.part;
      ui.say(`Saved ${name} simplified · ${count(view.curves_before)} → ${count(view.curves_after)} lines and arcs`);
    } catch (e) {
      error = explain(e);
    } finally {
      saving = false;
    }
  }
  /** What else was taken out: repeated contours and specks. */
  function dropped(view: SimplifyView): string {
    return [view.repeats ? `${view.repeats} repeated` : '', view.specks ? `${view.specks} too small` : ''].filter(Boolean).join(' · ');
  }
</script>

<Modal title="Simplify drawing" onclose={() => { if (!saving) onclose(); }}>
  <p class="muted">Joins runs of short lines, as CAD exports flatten curves, into the arcs and lines they follow, removes contours that repeat
    another on the same layer, and drops specks. Nothing moves further than the tolerance. It is saved as a new part; {part.name} stays as it is
    for the jobs that use it.</p>
  <div class="tolerances" role="group" aria-label="Tolerance">
    {#each TOLERANCES as [label, value]}
      <button aria-pressed={tolerance === value} disabled={saving} onclick={() => (tolerance = value)}><strong>{label}</strong><small>{quantity(value, 'mm', 3)}</small></button>
    {/each}
  </div>
  <dl class="outcome" aria-live="polite">
    {#if result}
      <dt>Lines and arcs</dt><dd>{count(result.curves_before)} → <strong>{count(result.curves_after)}</strong></dd>
      <dt>Contours</dt><dd>{count(result.contours_before)} → <strong>{count(result.contours_after)}</strong>{#if result.repeats || result.specks}<small
        >{dropped(result)}</small>{/if}</dd>
    {:else if checking}
      <dt>Checking…</dt><dd></dd>
    {/if}
  </dl>
  {#if result && !changed}<p class="gate-reason">Already as simple as this tolerance allows.</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="actions">
    <button class="btn btn-ghost lg" disabled={saving} onclick={onclose}>Cancel</button>
    <button class="btn btn-primary lg" disabled={saving || checking || !changed} onclick={save}>{saving ? 'Saving…' : 'Save as new part'}</button>
  </div>
</Modal>

<style>
  .muted { margin: 0 0 16px; line-height: 1.6; }
  .tolerances { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; }
  .tolerances button { min-height: 64px; display: grid; gap: 4px; place-content: center; border: 1px solid var(--line); border-radius: 10px; background: var(--panel-2); color: var(--ink); cursor: pointer; }
  .tolerances button[aria-pressed="true"] { border-color: var(--accent); color: var(--accent); background: var(--accent-soft); }
  .tolerances small { color: var(--ink-3); font-size: var(--t-sm); }
  .outcome { display: grid; grid-template-columns: auto 1fr; gap: 10px 16px; margin: 20px 0 8px; min-height: 52px; }
  .outcome dt { color: var(--ink-3); }
  .outcome dd { margin: 0; display: flex; flex-wrap: wrap; gap: 4px 10px; align-items: baseline; }
  .outcome small { color: var(--ink-3); font-size: var(--t-sm); }
  .error { color: var(--warn); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
</style>
