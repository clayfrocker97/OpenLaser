<script lang="ts">
  import MachineFiles from './MachineFiles.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { settingsEdits } from '../lib/settings-edits.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain } from '../lib/format';
  import { fieldId, fieldUnit, xmlValue, groupKey, groupName, isLayer, readable, sectionName, type Comparison, type MachineSettings, type XmlField } from '../lib/machine-settings';
  import { inputValue, sourceInput, unitLabel, units } from '../lib/units.svelte';

  let data = $state<MachineSettings | null>(null);
  let busy = $state(false);
  let error = $state('');
  let query = $state('');
  let group = $state('/ParameterRoot/PMachineAxisConfig_0');
  let filter = $state<'all' | 'mismatch' | 'edited'>('all');
  const draft = $derived(settingsEdits.entries.xml);
  const changed = $derived(Object.keys(draft?.edits ?? {}).length);
  const stale = $derived(!!draft && !!data && draft.base !== data.sha256);
  const connected = $derived(server.doc?.machine.connection.state === 'connected');
  const machineBusy = $derived(!!server.doc?.machine.operation);
  const controllerWorking = $derived(server.doc?.machine.operation?.kind === 'parameters' || server.doc?.link.phase === 'reading');
  const comparisons = $derived.by(() => {
    const map = new Map<string, Comparison[]>();
    for (const comparison of data?.comparisons ?? []) {
      for (const field of comparison.fields) map.set(field, [...(map.get(field) ?? []), comparison]);
    }
    return map;
  });
  const mismatches = $derived((data?.comparisons ?? []).filter(c => c.actual !== null && c.actual !== c.expected));
  const mismatchCount = $derived(new Set(mismatches.map(c => c.address)).size);
  const fields = $derived((data?.fields ?? []).filter(f => !isLayer(f.path)));
  const groups = $derived([...new Set(fields.map(f => groupKey(f.path)))].map(key => ({ key, name: groupName(key), count: fields.filter(f => groupKey(f.path) === key).length })));
  const matches = (f: XmlField): boolean => {
    const id = fieldId(f);
    const search = query.trim().toLocaleLowerCase();
    if (search && !`${readable(f.name, f.path)} ${f.path} ${f.name} ${f.value} ${groupName(f.path)}`.toLocaleLowerCase().includes(search)) return false;
    if (filter === 'edited' && !draft?.edits[id]) return false;
    if (filter === 'mismatch' && !comparisons.get(id)?.some(c => c.actual !== null && c.actual !== c.expected)) return false;
    return true;
  };
  const shown = $derived(fields.filter(f => (query.trim() || filter !== 'all' || groupKey(f.path) === group) && matches(f)));
  const grouped = $derived.by(() => {
    const sections = new Map<string, { path: string; title: string; fields: XmlField[] }>();
    for (const field of shown) {
      const title = sectionName(field);
      const key = `${field.path}#${title}`;
      if (!sections.has(key)) sections.set(key, { path: field.path, title, fields: [] });
      sections.get(key)!.fields.push(field);
    }
    return [...sections.values()];
  });
  const value = (f: XmlField): string => draft?.edits[fieldId(f)]?.value ?? f.value;
  const effective = $derived((data?.fields ?? []).map(f => ({ ...f, value: value(f) })));
  const number = (v: number): string => v.toLocaleString('en-US');

  async function refresh(): Promise<void> {
    data = await api.machineSettings();
    if (!fields.some(f => groupKey(f.path) === group)) group = fields[0] ? groupKey(fields[0].path) : '';
  }
  async function run(action: () => Promise<unknown>): Promise<void> {
    if (busy) return;
    busy = true; error = '';
    try { await action(); }
    catch (e) { error = explain(e); }
    finally { try { await refresh(); } catch (e) { error ||= explain(e); } busy = false; }
  }
  let loadedState = $state('');
  $effect(() => {
    const doc = server.doc;
    const state = `${doc?.files.backup?.sha256}:${doc?.machine.connection.state}:${doc?.machine.session.parameters_verified}:${doc?.machine.operation?.kind}:${doc?.link.phase}:${doc?.message?.id}`;
    // Refresh when initialization, reconnect, another tab's write or a live
    // parameter change finishes. Keep a refresh pending while a local action runs.
    if (!busy && state !== loadedState) { loadedState = state; void run(async () => {}); }
  });
  function edit(f: XmlField): void {
    if (!data) return;
    const hash = data.sha256;
    const current = value(f);
    const numeric = current.trim() !== '' && Number.isFinite(Number(current));
    const source = fieldUnit(f, effective), system = units.system;
    osk.show({ kind: numeric ? 'num' : 'text', label: readable(f.name, f.path), unit: unitLabel(source, system), value: numeric && source ? inputValue(Number(current), source, system) : current, fresh: numeric,
      onCommit: text => {
        const n = sourceInput(text, Number(current), source, system);
        if (numeric && !Number.isFinite(n)) return;
        const next = numeric ? n === Number(current) ? current : String(n) : text;
        void settingsEdits.xml(f, next, hash);
      } });
  }
  async function save(): Promise<void> {
    if (!draft || stale) return;
    const copy = $state.snapshot(draft);
    await api.saveMachineSettings(copy.base, Object.values(copy.edits));
    await settingsEdits.discard('xml', true);
  }
  async function write(): Promise<void> {
    if (draft) await save();
    else if (data) await api.writeMachineSettings(data.sha256);
  }
  /** Writing replaces the controller's parameters: say what, then ask. */
  function confirmWrite(): Promise<boolean> {
    return ui.confirm({
      title: 'Write the controller?',
      body: changed
        ? `${changed} changed value${changed === 1 ? ' is' : 's are'} saved, then every machine setting is written to the controller and read back to verify it.`
        : 'Every machine setting from the backup is written to the controller and read back to verify it.',
      confirm: 'Write controller',
    });
  }
</script>

<div class="xml-settings">
  <div class="xml-summary">
    <div><h2>Machine settings</h2><div class="xml-meta"><p>{data?.name ?? 'Machine backup'} · {fields.length.toLocaleString()} values</p>
      <span class="status-pill" role="status" class:mismatch={!controllerWorking && mismatches.length > 0} class:matched={!controllerWorking && data?.connected && !mismatches.length && !data?.problem}>{controllerWorking ? 'Updating controller…' : data?.connected ? mismatches.length ? `${mismatchCount} ${mismatchCount === 1 ? 'mismatch' : 'mismatches'}` : data.problem ? 'Initialization needs attention' : 'Controller matches' : 'Controller not read'}</span>
    </div></div>
    <div class="xml-actions">
      <MachineFiles disabled={busy || machineBusy} />
      <button class="btn btn-ghost" disabled={busy || machineBusy || !connected} onclick={() => run(api.readMachineSettings)}>Read controller</button>
      <button class="btn btn-primary" disabled={busy || machineBusy || !data || !connected || stale} onclick={async () => { if (await confirmWrite()) run(write); }}>{busy ? 'Working…' : changed ? 'Save & write controller' : 'Write controller'}</button>
    </div>
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if data?.problem}<details class="xml-problem"><summary>Controller comparison details</summary><p>{data.problem}</p></details>{/if}
  {#if changed}
    <div class="draft-bar"><div><strong>{changed} edited {changed === 1 ? 'value' : 'values'}</strong><small>{stale ? 'Backup changed; review your edits.' : connected ? 'Save applies changes to the controller.' : 'Applied on next connection.'}</small></div>
      <div class="xml-actions"><button class="btn btn-ghost" disabled={busy} onclick={async () => { if (await ui.confirm({ title: 'Discard these edits?', body: `${changed} edited machine ${changed === 1 ? 'value is' : 'values are'} thrown away.`, confirm: 'Discard edits', danger: true })) run(() => settingsEdits.discard('xml')); }}>Discard edits</button><button class="btn btn-primary" disabled={busy || machineBusy || stale} onclick={async () => { if (!connected || await confirmWrite()) run(save); }}>{connected ? 'Save & apply' : 'Save XML'}</button></div>
    </div>
  {/if}
  <div class="xml-tools">
    <input class="xml-search" aria-label="Search machine settings" type="search" bind:value={query} placeholder="Search settings…" />
    <div class="seg">{#each [['all', 'All values'], ['mismatch', 'Mismatches'], ['edited', 'Edited']] as [key, label]}<button class:on={filter === key} onclick={() => filter = key as typeof filter}>{label}</button>{/each}</div>
  </div>
  <div class="xml-body">
    <nav class="xml-nav" aria-label="XML setting groups">
      {#each groups as item}<button class:active={item.key === group && !query && filter === 'all'} onclick={() => { group = item.key; query = ''; filter = 'all'; }}><span>{item.name}</span><small>{item.count}</small></button>{/each}
    </nav>
    <div class="xml-values">
      <div class="values-heading"><strong>{query.trim() ? 'Search results' : filter === 'mismatch' ? 'Mismatched XML settings' : filter === 'edited' ? 'Pending XML edits' : groupName(group)}</strong><span>{shown.length} {shown.length === 1 ? 'value' : 'values'}</span></div>
      {#each grouped as section}
        <section class="xml-group">
          <h3>{section.title}</h3>
          {#each section.fields as f (fieldId(f))}
            {@const checks = comparisons.get(fieldId(f)) ?? []}
            {@const mismatch = checks.some(c => c.actual !== null && c.actual !== c.expected)}
            {@const edited = !!draft?.edits[fieldId(f)]}
            <div class="xml-row" class:row-mismatch={mismatch} class:row-edited={edited}>
              <div class="xml-label"><strong>{readable(f.name, f.path)}</strong><code>{f.name}</code>{#if edited}<small>Saved: {xmlValue(f, f.value, data?.fields)}</small>{/if}</div>
              <button class="xml-value" disabled={busy} aria-label={`Edit ${readable(f.name, f.path)} (${f.name})`} onclick={() => edit(f)}><span>{xmlValue(f, value(f), effective)}</span><span class="edit-mark" aria-hidden="true">↗</span></button>
              <div class="xml-readback" class:warn={mismatch}>
                {#if edited}<strong>Edited · pending save</strong>{/if}
                {#if !checks.length}<span class="muted">XML value</span><small>No controller readback</small>
                {:else if !data?.connected}<span class="muted">Controller not read</span>
                {:else}<strong>{edited ? mismatch ? 'Saved value differs' : 'Saved value matched' : mismatch ? 'Mismatch' : 'Matched'}</strong>{/if}
                {#if checks.length}<details><summary>Controller words</summary>{#each checks as c}<div class="word"><code>Register {c.address}</code><span>Expected {number(c.expected)}</span><span>Read {c.actual === null ? '—' : number(c.actual)}</span>{#if c.mask !== 4294967295}<small>Mask 0x{c.mask.toString(16)}</small>{/if}</div>{/each}</details>{/if}
              </div>
            </div>
          {/each}
        </section>
      {:else}<div class="xml-empty">{busy ? 'Loading machine settings…' : data ? 'No settings match this filter.' : 'Import your backup XML to edit its settings here.'}</div>{/each}
    </div>
  </div>
</div>

<style>
  .xml-settings { min-width: 0; display: grid; gap: 12px; }
  .xml-summary, .xml-tools, .xml-actions, .xml-meta, .draft-bar, .values-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  h2 { margin: 0 0 5px; font-size: var(--t-lg); } p { margin: 0; } .xml-summary p, .values-heading span { color: var(--ink-3); font-size: var(--t-sm); }
  .xml-meta { justify-content: flex-start; gap: 8px 14px; }
  .status-pill { font-size: var(--t-sm); font-weight: 600; color: var(--ink-3); white-space: nowrap; } .matched { color: var(--ok); } .mismatch, .warn { color: #b86112; }
  .error, .xml-problem { color: var(--warn); font-size: var(--t-sm); overflow-wrap: anywhere; }
  .xml-problem p { margin-top: 8px; line-height: 1.6; }
  .draft-bar { padding: 12px 14px; background: color-mix(in srgb, var(--accent) 7%, transparent); border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent); border-radius: 8px; }
  .draft-bar small { display: block; margin-top: 4px; color: var(--ink-3); }
  .xml-search { background: var(--panel-2); color: var(--ink); border: 1px solid var(--line); border-radius: 8px; padding: 10px 12px; font-size: var(--t-base); flex: 1; min-width: 230px; min-height: 44px; }
  .xml-tools .seg button { min-height: 44px; }
  .xml-body { display: grid; grid-template-columns: 176px minmax(0, 1fr); gap: 22px; align-items: start; }
  .xml-nav { display: grid; gap: 3px; max-height: 67vh; overflow: auto; padding-right: 5px; position: sticky; top: 0; }
  .xml-nav > button { background: transparent; cursor: pointer; width: 100%; min-height: 44px; text-align: left; display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 10px; color: var(--ink-3); border: 1px solid transparent; border-radius: 6px; font-size: var(--t-sm); line-height: 1.4; }
  .xml-nav > button.active { color: var(--ink); background: color-mix(in srgb, var(--accent) 9%, transparent); border-color: color-mix(in srgb, var(--accent) 20%, transparent); font-weight: 600; }
  .xml-nav > button:hover { background: color-mix(in srgb, var(--accent) 6%, transparent); }
  .xml-nav small { font-size: var(--t-sm); opacity: .75; }
  .xml-values { min-width: 0; } .values-heading { margin-bottom: 12px; } .values-heading strong { font-size: var(--t-base); }
  .xml-group { border: 1px solid var(--line); border-radius: 8px; margin-bottom: 16px; overflow: hidden; }
  .xml-group h3 { text-transform: none; letter-spacing: normal; color: var(--ink-2); margin: 0; padding: 12px 14px; font-size: var(--t-sm); }
  .xml-row { display: grid; grid-template-columns: minmax(130px, 1.25fr) minmax(90px, .85fr) minmax(118px, .9fr); gap: 14px; align-items: center; border-top: 1px solid var(--line); padding: 12px 14px; min-height: 77px; }
  .row-mismatch { background: color-mix(in srgb, #d98930 7%, transparent);  }
  .row-edited { background: var(--accent-soft); }
  .xml-label { min-width: 0; } .xml-label strong { font-size: var(--t-base); line-height: 1.5; display: block; font-weight: 600; }
  .xml-label code { display: block; margin-top: 3px; font-size: var(--t-sm); color: var(--ink-3); overflow-wrap: anywhere; }
  .xml-label small { display: block; font-size: var(--t-sm); color: var(--accent); margin-top: 5px; overflow-wrap: anywhere; }
  .xml-value { background: var(--panel-2); cursor: pointer; min-width: 0; display: flex; gap: 8px; align-items: center; justify-content: space-between; min-height: 44px; border: 1px solid var(--line); border-radius: 6px; padding: 9px 10px; text-align: left; font-size: var(--t-sm); font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
  .xml-value:hover { border-color: var(--accent); } .xml-value > span:first-child { min-width: 0; max-height: 76px; overflow: auto; } .edit-mark { color: var(--ink-3); flex: none; }
  .xml-readback { min-width: 0; font-size: var(--t-sm); display: grid; gap: 4px; line-height: 1.4; } .xml-readback strong { font-weight: 600; } .xml-readback small { font-size: var(--t-sm); color: var(--ink-3); }
  .xml-readback summary { cursor: pointer; color: var(--ink-3); padding: 5px 0; font-size: var(--t-sm); } .word { display: grid; gap: 3px; margin: 6px 0; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; } .word code { font-size: var(--t-sm); }
  .xml-empty { padding: 45px 15px; color: var(--ink-3); text-align: center; }
  @media (max-width: 1120px) { .xml-body { grid-template-columns: 150px minmax(0, 1fr); gap: 12px; } .xml-row { grid-template-columns: minmax(100px, 1fr) minmax(90px, 1fr); gap: 8px; } .xml-readback { grid-column: 1 / -1; display: flex; gap: 8px; flex-wrap: wrap; align-items: center; } }
  @media (max-width: 700px) { .xml-body { grid-template-columns: 1fr; } .xml-nav { max-height: 200px; grid-template-columns: repeat(2, 1fr); position: static; }  }
</style>
