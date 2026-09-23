<script lang="ts">
  // The import review. Every recipe file of the chosen files, folders or
  // drop is read without saving, then listed: the material it goes under
  // (matched by name and thickness, changeable, or a new one), its
  // thickness, the nozzle, focus and lens read from its note and names
  // for the operator to confirm, the shared summary, and, for a recipe
  // the library already has at that material, thickness and gas, a choice
  // of Replace, Keep both or Skip, one at a time or for all. Nothing is
  // saved until Import; replacing recipes asks first (DESIGN.md rule 7).
  import { onDestroy } from 'svelte';
  import Modal from '../../components/Modal.svelte';
  import MaterialSummary from '../../components/MaterialSummary.svelte';
  import { api } from '../../api/client';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { explain, laserLabel, plural } from '../../lib/format';
  import { inputValue, quantity, sourceInput, unitLabel, units } from '../../lib/units.svelte';
  import { materialsOf } from '../../lib/materials';
  import { num, refused, shown } from '../../lib/recipe';
  import {
    MAX_FILES, choicesFor, duplicatesOf, importRequest, needsChoice, recipeFiles, reviewItem,
    type Choice, type ImportFailure, type ImportItem, type Picked,
  } from '../../lib/recipe-import';
  import type { HeadSetup, RecipeImport } from '../../api';

  let { files, onclose }: { files: Picked[]; onclose: () => void } = $props();

  const doc = $derived(server.doc!);
  const recipes = $derived(doc.library.recipes);
  let items = $state<ImportItem[]>([]);
  let failures = $state<ImportFailure[]>([]);
  let phase = $state<'reading' | 'review' | 'saving'>('reading');
  let progress = $state({ done: 0, total: 0 });
  let skippedOver = $state(0);
  let closed = false;
  onDestroy(() => { closed = true; });

  // Read every recipe file once, in order.
  async function read(): Promise<void> {
    const found = recipeFiles(files);
    skippedOver = Math.max(0, found.length - MAX_FILES);
    const chosen = found.slice(0, MAX_FILES);
    progress = { done: 0, total: chosen.length };
    for (const [i, entry] of chosen.entries()) {
      if (closed) return;
      try {
        const preview = await api.previewRecipe(entry.file.name, await entry.file.arrayBuffer());
        items.push(reviewItem(`${i}:${entry.path}`, entry.file, entry.photo, preview, recipes));
      } catch (error) {
        failures.push({ name: entry.path, reason: explain(error) });
      }
      progress.done += 1;
    }
    phase = 'review';
  }
  read();

  const rows = $derived(items.map((item) => ({
    item,
    dup: duplicatesOf(item, items, recipes),
    choices: choicesFor(item, items, recipes),
    needs: needsChoice(item, items, recipes),
  })));
  const duplicates = $derived(rows.filter((r) => r.needs));
  const unresolved = $derived(duplicates.filter((r) => r.item.choice === null).length);
  const plan = $derived(items.map((item) => ({ item, request: importRequest(item, items, recipes) })));
  const saving = $derived(plan.filter((p) => p.request));
  const replacing = $derived(saving.filter((p) => p.request?.replace).length);
  const newMaterials = $derived([...new Set(items.filter((i) => i.isNew && importRequest(i, items, recipes)).map((i) => i.material))]);
  /** The notes after the file count in the heading, each led by a dot. */
  const headNotes = $derived([
    duplicates.length ? `${duplicates.length} already in the library` : '',
    failures.length ? `${failures.length} unreadable` : '',
    newMaterials.length ? `new material${newMaterials.length === 1 ? '' : 's'}: ${newMaterials.join(', ')}` : '',
  ].filter(Boolean).map((part) => ` · ${part}`).join(''));
  const importLabel = $derived(
    saving.length ? `Import ${saving.length}${replacing ? ` · replace ${replacing}` : ''}` : 'Nothing to import',
  );
  const LABEL: Record<Choice, string> = { replace: 'Replace', keep: 'Keep both', skip: 'Skip' };
  const CHOICES: Choice[] = ['replace', 'keep', 'skip'];

  const materialsFor = (laser: string) => materialsOf(recipes).filter((m) => m.laser === laser).map((m) => m.name);
  function chooseMaterial(item: ImportItem, select: HTMLSelectElement): void {
    const value = select.value;
    if (value === '\u0000new') {
      select.value = item.material;
      osk.text('New material', item.material, (name) => {
        if (name.trim()) {
          item.material = name.trim();
          item.isNew = !materialsFor(item.preview.laser).includes(item.material);
          item.choice = null;
        }
      });
      return;
    }
    item.material = value;
    item.isNew = !materialsFor(item.preview.laser).includes(value);
    item.choice = null;
  }
  function thickness(item: ImportItem): void {
    osk.number('Thickness', item.thickness_mm, 'mm', (v) => { if (v >= 0) { item.thickness_mm = v; item.choice = null; } });
  }
  /** Types a setup value; a blank entry clears it. */
  function setup(item: ImportItem, key: 'nozzle_diameter_mm' | 'focus_mm' | 'lens_mm', attribute: string, label: string): void {
    const system = units.system;
    const current = item.setup[key];
    const ask = (start: string) => osk.show({
      kind: 'num', label, value: start, unit: unitLabel('mm', system), fresh: true,
      onCommit: (text) => {
        if (text.trim() === '') { item.setup[key] = null; return; }
        const n = sourceInput(text, Number(current), 'mm', system);
        const why = refused(attribute, n);
        if (why) { ui.say(`${label}: ${why}`, true); ask(text); return; }
        item.setup[key] = current !== null && n === Number(current) ? current : num(n);
      },
    });
    ask(current === null ? '' : inputValue(Number(current), 'mm', system));
  }
  function nozzle(item: ImportItem, kind: HeadSetup['nozzle']): void { item.setup.nozzle = kind; }
  function all(choice: Choice): void {
    for (const r of duplicates) r.item.choice = r.choices.includes(choice) ? choice : 'keep';
  }
  /** The review's summary: the item's own setup over the file's values. */
  const source = (item: ImportItem) => ({
    laser: item.preview.laser,
    gas: item.preview.gas,
    summary: { ...item.preview.summary, setup: item.setup },
  });
  const shownValue = (key: string, value: string | null) => (value === null ? 'Not set' : shown(key, value));
  /** Why a row is a duplicate: the same file, a library recipe, or an earlier file of this import. */
  function dupText(item: ImportItem, dup: ReturnType<typeof duplicatesOf>): string {
    if (dup.identical) return 'This exact file is already in the library';
    const what = `${item.material} ${quantity(item.thickness_mm, 'mm')} ${item.preview.gas}`;
    return dup.recipes.length ? `${what} is already in the library` : `An earlier file in this import is also ${what}`;
  }

  async function save(): Promise<void> {
    if (unresolved || !saving.length) return;
    if (replacing) {
      const ok = await ui.confirm({
        title: `Replace ${plural(replacing, 'recipe')}?`,
        body: `${replacing === 1 ? 'Its' : 'Their'} values, note and source file are replaced by the imported file;`
          + ' the name, star, photo and film process stay. Library history can undo it.',
        confirm: `Replace ${replacing}`,
        danger: true,
      });
      if (!ok) return;
    }
    // The plan is fixed before the first save changes the library.
    const steps: Array<{ item: ImportItem; request: RecipeImport }> = saving.map((p) => ({ item: p.item, request: p.request! }));
    phase = 'saving';
    progress = { done: 0, total: steps.length };
    const tally = { added: 0, replaced: 0, known: 0, failed: [] as ImportFailure[] };
    for (const { item, request } of steps) {
      try {
        const { id, existing } = await api.importRecipe(request, await item.file.arrayBuffer());
        if (existing) tally.known += 1;
        else if (request.replace) tally.replaced += 1;
        else tally.added += 1;
        if (item.photo && !existing) await api.setPhoto(id, await item.photo.arrayBuffer());
      } catch (error) {
        tally.failed.push({ name: item.file.name, reason: explain(error) });
      }
      progress.done += 1;
    }
    const skipped = items.length - steps.length;
    const known = tally.known ? `, ${tally.known} already there` : '';
    const failed = tally.failed.length ? `, ${tally.failed.length} failed` : '';
    ui.say(`${tally.added} added, ${tally.replaced} replaced${known}${skipped ? `, ${skipped} skipped` : ''}${failed}.`, tally.failed.length > 0);
    if (tally.failed.length) {
      failures = tally.failed;
      items = [];
      phase = 'review';
    } else onclose();
  }
</script>

<Modal title="Import recipes" wide dismissable={phase !== 'saving'} {onclose}>
  {#if phase === 'reading'}
    <p class="muted" role="status">Reading {progress.done} of {progress.total} recipe files…</p>
  {:else if phase === 'saving'}
    <p class="muted" role="status">Saving {progress.done} of {progress.total}…</p>
  {:else}
    <div class="import-head">
      <p><strong>{plural(items.length, 'recipe file')}</strong>{headNotes}</p>
      <p class="muted">Check each material and the nozzle, focus and lens read from the notes and file names. Nothing is saved until Import.</p>
      {#if skippedOver}<p class="warn-text">Only the first {MAX_FILES} recipe files are listed; import the other {skippedOver} separately.</p>{/if}
    </div>
    {#if duplicates.length > 1}
      <div class="apply-all" role="group" aria-label="Every duplicate">
        <span>Every duplicate</span>
        {#each CHOICES as choice (choice)}<button class="btn btn-ghost" onclick={() => all(choice)}>{LABEL[choice]} all</button>{/each}
      </div>
    {/if}
    {#if !items.length && !failures.length}<p class="muted">No recipe files (.xml) were found.</p>{/if}
    <ul class="import-list">
      {#each rows as { item, dup, choices, needs } (item.key)}
        <li class="import-item" class:dup={needs} class:skip={item.choice === 'skip' && needs}>
          <div class="file-line">
            <span class="file">{item.preview.file_name}</span>
            <span class="tag {item.preview.laser}">{laserLabel(item.preview.laser)}</span>
            <span class="tag">{item.preview.gas}</span>
            {#if item.photo}<span class="tag">Photo</span>{/if}
          </div>
          <div class="identity">
            <label class="pick"><span>Material{#if item.isNew}<b class="new">New</b>{/if}</span>
              <select value={item.material} onchange={(e) => chooseMaterial(item, e.currentTarget)}>
                {#if item.isNew}<option value={item.material}>{item.material} (new)</option>{/if}
                {#each materialsFor(item.preview.laser) as name (name)}<option value={name}>{name}</option>{/each}
                <option value={'\u0000new'}>New material…</option>
              </select>
            </label>
            <button class="value" data-numpad onclick={() => thickness(item)}>
              <span>Thickness</span>
              <b>{item.thickness_mm > 0 ? quantity(item.thickness_mm, 'mm') : 'Not set'}</b>
            </button>
          </div>
          <div class="setup" aria-label="Head setup read from the file">
            <button class="value" data-numpad onclick={() => setup(item, 'nozzle_diameter_mm', 'OpenLaserNozzleDiameter', 'Nozzle diameter')}>
              <span>Nozzle</span>
              <b>{shownValue('OpenLaserNozzleDiameter', item.setup.nozzle_diameter_mm)}</b>
            </button>
            <div class="seg nozzle" role="group" aria-label="Nozzle type">
              <button class:on={item.setup.nozzle === 'single'} onclick={() => nozzle(item, 'single')}>Single</button>
              <button class:on={item.setup.nozzle === 'double'} onclick={() => nozzle(item, 'double')}>Double</button>
              <button class:on={item.setup.nozzle === null} onclick={() => nozzle(item, null)}>Not set</button>
            </div>
            <button class="value" data-numpad onclick={() => setup(item, 'focus_mm', 'OpenLaserManualFocus', 'Focus')}>
              <span>Focus</span>
              <b>{shownValue('OpenLaserManualFocus', item.setup.focus_mm)}</b>
            </button>
            <button class="value" data-numpad onclick={() => setup(item, 'lens_mm', 'OpenLaserLens', 'Lens')}>
              <span>Lens</span>
              <b>{shownValue('OpenLaserLens', item.setup.lens_mm)}</b>
            </button>
          </div>
          <MaterialSummary source={source(item)} variant="line" />
          {#if needs}
            <div class="dup-line">
              <p>
                <i class="ic ic-copy"></i>{dupText(item, dup)}{#if dup.recipes[0]}<span class="theirs">Library: <MaterialSummary
                  source={dup.recipes[0]} variant="line" /></span>{/if}
              </p>
              <div class="seg choice" role="group" aria-label="What to do with the duplicate">
                {#each choices as choice (choice)}
                  <button class:on={item.choice === choice} onclick={() => (item.choice = choice)}>{LABEL[choice]}</button>
                {/each}
              </div>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
    {#if failures.length}
      <details class="failures" open={!items.length}>
        <summary>{plural(failures.length, 'file')} could not be read</summary>
        <div class="import-log">{failures.map((f) => `${f.name}: ${f.reason}`).join('\n')}</div>
      </details>
    {/if}
    <div class="import-foot">
      {#if unresolved}<p class="gate-reason">Choose Replace, Keep both or Skip for {plural(unresolved, 'duplicate')}.</p>{/if}
      <div class="row">
        <button class="btn btn-ghost" onclick={onclose}>Cancel</button>
        <button class="btn btn-primary" disabled={!!unresolved || !saving.length} onclick={save}>{importLabel}</button>
      </div>
    </div>
  {/if}
</Modal>

<style>
  .import-head p { margin: 0 0 4px; }
  .apply-all { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .apply-all span { font-size: var(--t-sm); color: var(--ink-2); margin-right: 4px; }
  .import-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 10px; }
  .import-item { display: flex; flex-direction: column; gap: 10px; padding: 12px; border: 1px solid var(--line); border-radius: 12px; background: var(--panel); }
  .import-item.dup { border-color: var(--warn); }
  .import-item.skip { opacity: .6; }
  .file-line { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .file { font-weight: 600; overflow-wrap: anywhere; min-width: 0; flex: 1 1 200px; }
  .file-line .tag { font-size: var(--t-sm); }
  .identity, .setup { display: flex; flex-wrap: wrap; gap: 8px; }
  .pick { display: flex; flex-direction: column; align-items: stretch; text-align: left; gap: 2px; flex: 1 1 220px; font-size: var(--t-sm); color: var(--ink-3); }
  .pick select { min-height: var(--touch); padding: 0 12px; border-radius: 12px; border: 1px solid var(--line); background: var(--panel-2); font: 600 var(--t-base) var(--font); color: var(--ink); }
  .new { margin-left: .4em; color: var(--accent-2); font-weight: 700; }
  .value { display: flex; flex-direction: column; justify-content: center; align-items: flex-start; min-height: var(--touch); min-width: 110px; padding: 4px 12px; border-radius: 12px; border: 1px solid var(--line); background: var(--panel-2); cursor: pointer; text-align: left; }
  .value span { font-size: var(--t-sm); color: var(--ink-3); }
  .value b { font-size: var(--t-base); font-variant-numeric: tabular-nums; }
  .seg button { min-height: 44px; }
  .dup-line { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; padding-top: 8px; border-top: 1px solid var(--line); }
  .dup-line p { margin: 0; font-size: var(--t-sm); color: var(--ink-2); flex: 1 1 260px; }
  .theirs { display: block; margin-top: 4px; color: var(--ink-3); }
  .dup-line .ic { color: var(--warn); margin-right: 6px; vertical-align: -2px; }
  .failures summary { cursor: pointer; min-height: 44px; display: flex; align-items: center; font-size: var(--t-sm); color: var(--ink-2); }
  .import-foot { display: flex; flex-direction: column; gap: 8px; position: sticky; bottom: -20px; margin: 0 -20px -20px; padding: 12px 20px 16px; background: var(--panel); border-top: 1px solid var(--line); }
  .import-foot .row { display: flex; justify-content: flex-end; gap: 8px; }
</style>
