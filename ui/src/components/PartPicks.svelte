<script lang="ts">
  // The parts picked on the Parts page, to set up as one job or to add to
  // the open one. They are laid out side by side in the order picked.
  import Preview from '../mobile/Preview.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain, plural, size } from '../lib/format';
  import { livePicks, partsOf, togglePick } from '../lib/job-parts';

  /** The parts the page shows now, for picking them all at once. */
  let { shown = [] }: { shown?: string[] } = $props();
  const doc = $derived(server.doc!);
  const picks = $derived(livePicks(ui.partPicks ?? [], doc.library.parts));
  const picked = $derived(partsOf({ parts: picks }, doc.library.parts));
  const draft = $derived(doc.draft);
  const inDraft = $derived(draft ? picked.filter((part) => draft.parts.some((p) => p.id === part.id)) : []);
  const rest = $derived(shown.filter((id) => !picks.includes(id)));
  let busy = $state(false);

  async function run(action: () => Promise<unknown>, done: string): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      await action();
      ui.partPicks = null;
      ui.tab = 'setup';
      ui.say(done);
    } catch (error) {
      ui.say(explain(error), true);
    } finally {
      busy = false;
    }
  }

  const count = (n: number) => plural(n, 'part');
  const setUp = () => run(() => api.openParts(picks), picks.length > 1 ? `${count(picks.length)} side by side · Nest parts fills a sheet` : 'Part opened');
  const add = () => run(() => api.addParts(picks), `Added ${count(picks.length)} beside the sheet · Undo takes them off`);
</script>

<aside class="panel side picks">
  <h2>{picks.length ? `${count(picks.length)} picked` : 'Pick parts'}</h2>
  <p class="muted">{picks.length ? 'They open side by side, in this order, as one job.' : 'Tap the parts to cut together. Saved jobs keep their own parts.'}</p>
  {#if picked.length}
    <ol class="pick-list">
      {#each picked as part, i (part.id)}
        <li>
          <span class="order" aria-hidden="true">{i + 1}</span>
          <Preview outline={part.outline} label={part.name} small />
          <span class="name"><strong>{part.name}</strong><small>{size(part.bounds)} · {plural(part.contours, 'path')}</small></span>
          <button class="btn btn-ghost icon-only" aria-label="Unpick {part.name}" onclick={() => (ui.partPicks = togglePick(picks, part.id))}><i class="ic ic-x"></i></button>
        </li>
      {/each}
    </ol>
  {/if}
  <div class="side-foot">
    {#if rest.length}<button class="btn btn-ghost lg block" disabled={busy} onclick={() => (ui.partPicks = [...picks, ...rest])}>Pick all {plural(shown.length, 'part')} here</button>{/if}
    {#if draft}
      <button class="btn btn-ghost lg block add" disabled={busy || !picks.length || inDraft.length > 0} onclick={add}>Add to {draft.name}</button>
      {#if inDraft.length}<p class="gate-reason">{inDraft[0]!.name} is already in {draft.name}; copy it on the sheet for more.</p>{/if}
    {/if}
    <button class="btn btn-primary lg block" disabled={busy || !picks.length} onclick={setUp}>{picks.length > 1 ? `Set up job with ${count(picks.length)} →` : 'Set up job →'}</button>
    <button class="btn btn-ghost lg block" disabled={busy} onclick={() => (ui.partPicks = null)}>Cancel</button>
  </div>
</aside>

<style>
  .pick-list { list-style: none; margin: 0; padding: 0; display: grid; gap: 8px; overflow-y: auto; min-height: 0; }
  .pick-list li { display: grid; grid-template-columns: 28px 64px minmax(0, 1fr) 48px; align-items: center; gap: 10px; padding: 6px; border: 1px solid var(--line); border-radius: 12px; background: var(--panel); }
  .order { width: 28px; height: 28px; border-radius: 50%; display: grid; place-items: center; background: var(--accent); color: #fff; font-weight: 700; font-size: var(--t-sm); }
  .name { display: grid; gap: 2px; min-width: 0; }
  .name strong, .name small { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .name small { color: var(--ink-3); font-size: var(--t-sm); }
  .pick-list .icon-only { min-width: 48px; min-height: 48px; }
  .add { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
