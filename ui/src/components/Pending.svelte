<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from './Modal.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { recipeEdits } from '../lib/recipe-edits.svelte';
  import { settingsEdits, routeValues, mergePreferences, same, type SettingKey } from '../lib/settings-edits.svelte';
  import { holdSeconds } from '../lib/hold-confirm';
  import { explain, laserLabel } from '../lib/format';
  import { shown } from '../lib/recipe';
  import { conflictText, reviewText } from '../lib/review-values';
  import { xmlValue, type XmlField } from '../lib/machine-settings';
  import { actionName } from '../lib/preflight';
  import type { MergeReview, PendingDraft, PreflightPreferences, RecipeView } from '../api';

  let { onclose }: { onclose: () => void } = $props();
  const doc = $derived(server.doc!);
  const staged = $derived(settingsEdits.entries);
  let drafts = $state<PendingDraft[]>([]);
  let preferences = $state<PreflightPreferences | null>(null);
  let reviewing = $state<{ key: string; name: string; review: MergeReview; choices: Record<string, boolean> } | null>(null);
  let busy = $state(false);
  let loaded = $state(false);
  let error = $state('');
  let savedTheme = $state(settingsEdits.readTheme());
  const route = $derived(routeValues(doc));
  const routeConflicts = $derived(Object.entries(staged.route ?? {}).filter(([key, edit]) => edit.base !== route[key as keyof typeof route] && edit.value !== route[key as keyof typeof route]));
  const prefMerge = $derived(staged.preflight && preferences ? mergePreferences($state.snapshot(staged.preflight.base), $state.snapshot(staged.preflight.value), $state.snapshot(preferences)) : null);
  const labels: Record<SettingKey, string> = { route: 'Controller route', preflight: 'Checklist defaults', theme: 'Display theme', hold: 'Hold times', backup: 'Machine backup', soft: 'Process settings', xml: 'Machine XML settings' };
  // Another screen saved hold times after these were staged: choose first.
  const holdConflict = $derived(!!staged.hold && !same(doc.hold, staged.hold.base));
  let xmlFields = $state<XmlField[]>([]);
  const display = (v: unknown): string => reviewText(v);

  async function refresh(): Promise<void> {
    [drafts, preferences] = await Promise.all([api.pendingDrafts(), api.preflightPreferences()]);
    if (staged.xml) xmlFields = (await api.machineSettings()).fields;
    savedTheme = settingsEdits.readTheme();
    loaded = true;
  }
  async function run(action: () => Promise<unknown>): Promise<void> {
    if (busy) return;
    busy = true; error = '';
    try { await action(); await refresh(); }
    catch (e) { error = explain(e); }
    finally { busy = false; }
  }
  onMount(() => { void run(refresh); });

  async function open(draft: PendingDraft): Promise<void> {
    if (draft.problem) throw new Error(draft.problem);
    const active = doc.draft;
    if (active?.key === draft.key) return;
    ui.pendingJobName = null;
    await api.openRetained(draft.key);
  }
  async function saveDraft(draft: PendingDraft): Promise<void> {
    await open(draft);
    const name = ui.pendingJobName ?? draft.name;
    if (server.doc?.draft?.sheets?.pages.some(p => !p.job)) { await api.saveJob(name); ui.pendingJobName = null; return; }
    const review = await api.mergeReview(name);
    if (review.conflicts.length) { reviewing = { key: draft.key, name, review, choices: {} }; return; }
    await api.saveJob(review.name);
    ui.pendingJobName = null;
  }
  async function resolveJob(): Promise<void> {
    if (!reviewing) return;
    const { review, choices, name } = $state.snapshot(reviewing);
    const merged = await api.resolveMerge(review.token, choices, name);
    await api.saveJob(merged.name);
    reviewing = null; ui.pendingJobName = null;
  }
  async function saveRecipe(recipe: RecipeView): Promise<void> {
    if (recipeEdits.conflicts(recipe).length) throw new Error('Choose the material values to keep.');
    const save = recipeEdits.begin(recipe.id);
    if (!save) return;
    try { await api.updateRecipe(recipe.id, save.change); recipeEdits.finish(save, true); }
    catch (e) { recipeEdits.finish(save, false); throw e; }
  }
  /** Saving machine settings or a backup writes a connected controller: say so first. */
  function confirmSave(key: SettingKey): Promise<boolean> {
    if (key !== 'xml' && key !== 'backup') return Promise.resolve(true);
    const connected = doc.machine.connection.state === 'connected';
    const count = key === 'xml' ? Object.keys(staged.xml?.edits ?? {}).length : 0;
    return ui.confirm({
      title: key === 'xml' ? 'Save machine settings?' : 'Use this machine backup?',
      body: `${key === 'xml' ? `${count} changed value${count === 1 ? '' : 's'} replace the machine settings` : `${staged.backup?.name ?? 'The staged backup'} replaces the machine settings`}${connected ? ', and the connected controller is written and read back to verify them.' : '. The controller is written when it next connects.'}`,
      confirm: connected ? 'Save and write controller' : 'Save settings',
    });
  }

  async function saveSetting(key: SettingKey): Promise<void> {
    savedTheme = settingsEdits.readTheme();
    if (key === 'route' && staged.route) {
      if (routeConflicts.length) throw new Error('Choose the route values to keep.');
      await api.setRoute({ ...Object.fromEntries(Object.entries(staged.route).map(([k, e]) => [k, e.value])), expected: Object.fromEntries(Object.entries(staged.route).map(([k, e]) => [k, e.base])) });
    } else if (key === 'preflight' && staged.preflight) {
      const saved = await api.preflightPreferences();
      const merged = mergePreferences($state.snapshot(staged.preflight.base), $state.snapshot(staged.preflight.value), saved);
      preferences = saved;
      if (merged.conflicts.length) throw new Error('Choose the checklist values to keep.');
      await api.savePreflightPreferences(merged.value);
    } else if (key === 'hold' && staged.hold) {
      await api.saveHoldTimes($state.snapshot(staged.hold.value), $state.snapshot(staged.hold.base));
    } else if (key === 'xml' && staged.xml) {
      await api.saveMachineSettings(staged.xml.base, Object.values($state.snapshot(staged.xml.edits)));
    } else if ((key === 'backup' || key === 'soft') && staged[key]) {
      const file = staged[key];
      if (key === 'backup') await api.importMachineFile(file.name, file.bytes, file.base);
      else await api.importSoft(file.name, file.bytes, file.base);
    }
    await settingsEdits.discard(key, true);
  }
</script>
<Modal title="Pending changes" wide {onclose}>
  <p class="muted">Review and save each item.</p>
  {#if error}<p class="warn-text" role="alert">{error}</p>{/if}
  {#if doc.persistence_error || recipeEdits.storageError || settingsEdits.error}<p class="warn-text" role="alert">{doc.persistence_error ?? recipeEdits.storageError ?? settingsEdits.error}</p>{/if}
  {#if !loaded}<p class="muted">Loading retained edits…</p>{/if}
  {#if reviewing}
    <section class="pending-item"><h3>Resolve {reviewing.name}</h3>
      {#each reviewing.review.conflicts as conflict}
        <div class="conflict"><strong>{conflict.path}</strong><small>Base: {conflictText(conflict.base, conflict.path)}</small>
          <button class="choice" class:chosen={reviewing.choices[conflict.path] === true} onclick={() => { if (reviewing) reviewing.choices[conflict.path] = true; }}>Keep draft: {conflictText(conflict.draft, conflict.path)}</button>
          <button class="choice" class:chosen={reviewing.choices[conflict.path] === false} onclick={() => { if (reviewing) reviewing.choices[conflict.path] = false; }}>Keep saved: {conflictText(conflict.saved, conflict.path)}</button>
        </div>
      {/each}
      <div class="actions"><button class="btn btn-ghost" onclick={() => (reviewing = null)}>Back</button><button class="btn btn-primary" disabled={busy || reviewing.review.conflicts.some(c => reviewing?.choices[c.path] === undefined)} onclick={() => run(resolveJob)}>Resolve and save</button></div>
    </section>
  {/if}
  {#each drafts as draft (draft.key)}
    <section class="pending-item"><div class="row"><div><h3>{draft.name}</h3><small>{draft.job ? 'Job' : 'Unsaved job setup'}{draft.problem ? ` · ${draft.problem}` : !draft.can_save ? ' · choose a material before saving' : ''}</small></div>
      <div class="actions"><button class="btn btn-ghost" disabled={busy || !!draft.problem} onclick={() => run(async () => { await open(draft); ui.tab = 'setup'; onclose(); })}>Open</button><button class="btn btn-ghost" disabled={busy} onclick={() => run(async () => { await api.discardDraft(draft.key); reviewing = null; })}>Discard</button><button class="btn btn-primary" disabled={busy || !draft.can_save} onclick={() => run(() => saveDraft(draft))}>Save job</button></div></div>
    </section>
  {/each}
  {#each recipeEdits.pending as id (id)}
    {@const recipe = doc.library.recipes.find(r => r.id === id)}
    {@const conflicts = recipe ? recipeEdits.conflicts(recipe) : []}
    <section class="pending-item"><div class="row"><div><h3>{recipe?.name ?? 'Deleted material'}</h3><small>{recipeEdits.count(id)} staged material values</small></div><div class="actions"><button class="btn btn-ghost" disabled={busy} onclick={() => { ui.selectedRecipe = id; ui.tab = 'materials'; onclose(); }}>Open</button><button class="btn btn-ghost" disabled={busy} onclick={async () => { if (await ui.confirm({ title: 'Discard these material values?', body: `${recipeEdits.count(id)} staged values for ${recipe?.name ?? 'this material'} are thrown away.`, confirm: 'Discard values', danger: true })) recipeEdits.discard(id); }}>Discard</button><button class="btn btn-primary" disabled={busy || !recipe || !!conflicts.length} onclick={() => recipe && run(() => saveRecipe(recipe))}>Save</button></div></div>
      {#each conflicts as conflict}<div class="conflict"><strong>{conflict.key}</strong><small>Base: {shown(conflict.key, conflict.base)}</small><button class="choice" onclick={() => recipe && recipeEdits.resolve(recipe, conflict.key, true)}>Keep draft: {shown(conflict.key, conflict.draft)}</button><button class="choice" onclick={() => recipe && recipeEdits.resolve(recipe, conflict.key, false)}>Keep saved: {shown(conflict.key, conflict.saved)}</button></div>{/each}
      <details><summary>Review values</summary><pre>{JSON.stringify(Object.fromEntries(Object.entries(recipeEdits.attributes(id)).map(([key, value]) => [key, shown(key, value)])), null, 2)}{recipeEdits.film(id) === undefined ? '' : `\nFilm: ${recipeEdits.film(id) ?? 'none'}`}</pre></details>
    </section>
  {/each}
  {#each settingsEdits.pending as key (key)}
    <section class="pending-item"><div class="row"><h3>{labels[key]}</h3><div class="actions"><button class="btn btn-ghost" disabled={busy} onclick={async () => { if (await ui.confirm({ title: `Discard ${labels[key].toLowerCase()} edits?`, body: 'The staged changes on this screen are thrown away.', confirm: 'Discard edits', danger: true })) run(() => settingsEdits.discard(key)); }}>Discard</button><button class="btn btn-primary" disabled={busy || (key === 'route' && !!routeConflicts.length) || (key === 'hold' && holdConflict) || (key === 'preflight' && !!prefMerge?.conflicts.length) || ((key === 'backup' || key === 'soft') && staged[key]?.base !== (key === 'backup' ? doc.files.backup?.sha256 ?? '' : doc.soft.sha256 ?? ''))} onclick={async () => { if (await confirmSave(key)) run(() => saveSetting(key)); }}>Save</button></div></div>
      {#if key === 'theme'}<p>{staged.theme?.base} → {staged.theme?.value}</p>{#if staged.theme && savedTheme !== staged.theme.base && savedTheme !== staged.theme.value}<div class="conflict"><small>Saved theme: {savedTheme}</small><button class="choice" onclick={() => run(() => settingsEdits.rebaseTheme())}>Keep draft theme</button><button class="choice" onclick={() => run(() => settingsEdits.discard('theme'))}>Keep saved theme</button></div>{/if}
      {:else if key === 'route'}
        {#each Object.entries(staged.route ?? {}) as [field, edit]}<p>{field}: {edit.base || 'automatic'} → {edit.value}</p>{/each}
        {#each routeConflicts as [field, edit]}<div class="conflict"><strong>{field}</strong><small>Base: {edit.base}</small><button class="choice" onclick={() => run(() => settingsEdits.resolveRoute(field as keyof typeof route, route[field as keyof typeof route], true))}>Keep draft: {edit.value}</button><button class="choice" onclick={() => run(() => settingsEdits.resolveRoute(field as keyof typeof route, route[field as keyof typeof route], false))}>Keep saved: {route[field as keyof typeof route]}</button></div>{/each}
      {:else if key === 'preflight' && staged.preflight}
        {#each prefMerge?.conflicts ?? [] as conflict}<div class="conflict"><strong>{conflict.key}</strong><small>Base: {display(conflict.base)}</small><button class="choice" onclick={() => preferences && run(() => settingsEdits.resolvePreflight(conflict.key, $state.snapshot(preferences!), true))}>Keep draft: {display(conflict.draft)}</button><button class="choice" onclick={() => preferences && run(() => settingsEdits.resolvePreflight(conflict.key, $state.snapshot(preferences!), false))}>Keep saved: {display(conflict.saved)}</button></div>{/each}
        {#if prefMerge}<details><summary>Review checklists</summary>
          {#each [['Preflight', prefMerge.value], ['Pause', prefMerge.value.pause], ['Postflight', prefMerge.value.postflight]] as [phase, defaults]}
            {#each ['fiber', 'co2'] as mode}
              {@const list = (defaults as PreflightPreferences['postflight'])[mode as 'fiber' | 'co2']}
              <div class="checklist-review"><strong>{phase} · {laserLabel(mode as 'fiber' | 'co2')} · {list.enabled ? 'On' : 'Off'}</strong>
                {#if list.enabled}<ul>{#each list.steps as step}<li>{step.text}{#if step.action}<small>{actionName(step.action)}</small>{/if}</li>{/each}</ul>{/if}
              </div>
            {/each}
          {/each}
        </details>{/if}
      {:else if key === 'hold' && staged.hold}
        <p>Move or fire: {holdSeconds(staged.hold.base.move_ms)} → {holdSeconds(staged.hold.value.move_ms)} · Set origin: {holdSeconds(staged.hold.base.zero_ms)} → {holdSeconds(staged.hold.value.zero_ms)} · every screen</p>
        {#if holdConflict}<div class="conflict"><small>Saved on another screen: move or fire {holdSeconds(doc.hold.move_ms)} · set origin {holdSeconds(doc.hold.zero_ms)}</small><button class="choice" onclick={() => run(() => settingsEdits.rebaseHold($state.snapshot(doc.hold)))}>Keep these hold times</button><button class="choice" onclick={() => run(() => settingsEdits.discard('hold'))}>Keep saved hold times</button></div>{/if}
      {:else if key === 'xml' && staged.xml}
        <p>{Object.keys(staged.xml.edits).length} XML edits · applied on save when connected.</p>
        {#if staged.xml.base !== (doc.files.backup?.sha256 ?? '')}<p class="warn-text">The backup changed. Open Settings to compare or discard these edits.</p>{/if}
        <details><summary>Review values</summary>{#each Object.values(staged.xml.edits) as field}<p><code>{field.path}/@{field.name}</code><br>{xmlValue(field, field.original, xmlFields)} → {xmlValue(field, field.value, xmlFields)}</p>{/each}</details>
      {:else if (key === 'backup' || key === 'soft') && staged[key]}
        {@const file = staged[key]}{@const saved = key === 'backup' ? doc.files.backup?.sha256 ?? '' : doc.soft.sha256 ?? ''}
        <p>{file.name} · {(file.bytes.byteLength / 1024).toFixed(1)} kB</p>
        {#if file.base !== saved}<div class="conflict"><small>Base: {file.base || 'none'}<br>Saved: {saved || 'none'}</small><button class="choice" onclick={() => run(() => settingsEdits.rebaseFile(key, saved))}>Keep selected file</button><button class="choice" onclick={() => run(() => settingsEdits.discard(key))}>Keep saved file</button></div>{/if}
      {/if}
    </section>
  {/each}
  {#if ui.picking}<p class="muted">Finish or cancel point edits in Setup.</p>{/if}
  {#if loaded && !drafts.length && !recipeEdits.pending.length && !settingsEdits.pending.length}<p>No pending changes.</p>{/if}
</Modal>

<style>
  .pending-item { border-top: 1px solid var(--line); padding: 16px 0; display: grid; gap: 12px; }
  .row { display: flex; justify-content: space-between; gap: 16px; flex-wrap: wrap; align-items: center; }
  h3, p { margin: 0; } small { display: block; color: var(--ink-2); overflow-wrap: anywhere; }
  .conflict { display: grid; gap: 8px; background: var(--panel-2); padding: 12px; border-radius: 8px; }
  .checklist-review { margin: 14px 0; } .checklist-review ul { margin: 6px 0; padding-left: 22px; } .checklist-review li + li { margin-top: 6px; }
  .choice { text-align: left; padding: 10px; border: 1px solid var(--line); background: var(--panel); border-radius: 6px; overflow-wrap: anywhere; white-space: pre-wrap; }
  .choice.chosen { border-color: var(--accent); background: var(--accent-soft); }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 240px; overflow: auto; font-size: 12px; } summary { cursor: pointer; color: var(--ink-2); }
</style>
