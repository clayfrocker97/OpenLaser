<script lang="ts">
  import { distance, quantity } from '../lib/units.svelte';
  import { onMount } from 'svelte';
  import Modal from './Modal.svelte';
  import SheetShape from './SheetShape.svelte';
  import RemnantInspector from './RemnantInspector.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { explain, fmt, laserLabel } from '../lib/format';
  import type { SheetView } from '../api';
  let { onchoose }: { onchoose?: (sheet: SheetView) => Promise<void> } = $props();
  let items = $state<SheetView[]>([]), next = $state<string | null>(null);
  let remnants = $state(true), busy = $state(false), loaded = $state(false), error = $state('');
  let inspected = $state<string | null>(null), adding = $state(false), search = $state('');
  const recipe = $derived(server.doc?.draft?.recipe);
  const jobs = $derived((server.doc?.library.jobs ?? []).filter(j => !search.trim() || `${j.name} ${j.recipe.name}`.toLowerCase().includes(search.toLowerCase())).slice().reverse().slice(0, 60));
  function compatible(sheet: SheetView): boolean { return !recipe || (recipe.laser === sheet.mode && Math.abs(recipe.thickness_mm - sheet.thickness_mm) < 1e-6 && recipe.name.toLowerCase() === sheet.material.toLowerCase()); }
  async function load(more = false): Promise<void> {
    busy = true; error = '';
    try { const page = await api.sheets({ remnants, before: more ? next ?? undefined : undefined }); items = more ? [...items, ...page.items] : page.items; next = page.next; loaded = true; }
    catch (e) { error = explain(e); } finally { busy = false; }
  }
  onMount(() => { void load(); });
  async function choose(sheet: SheetView): Promise<void> {
    if (!onchoose) { inspected = sheet.id; return; }
    busy = true; error = '';
    try { await onchoose(sheet); } catch (e) { error = explain(e); } finally { busy = false; }
  }
  async function report(id: string): Promise<void> {
    busy = true; error = '';
    try { const result = await api.reportCutSheet(id); adding = false; inspected = result.id; remnants = false; await load(); }
    catch (e) { error = explain(e); } finally { busy = false; }
  }
</script>

<div class="sheet-library">
  {#if !onchoose}<div class="library-tools"><div class="seg"><button class:on={remnants} disabled={busy} onclick={() => { remnants = true; void load(); }}>Remnants</button><button class:on={!remnants} disabled={busy} onclick={() => { remnants = false; void load(); }}>Cut history</button></div><button class="btn btn-ghost" disabled={busy} onclick={() => adding = true}>Mark a saved job as cut</button></div>{:else}<p class="intro">Choose an inspected sheet. Its existing cutouts stay clear in the nest.</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="sheets-grid">{#each items as sheet}<button class="sheet-card" disabled={busy || (!!onchoose && !compatible(sheet))} onclick={() => choose(sheet)}><div class="shape"><SheetShape outline={sheet.outline} cutouts={sheet.cutouts} label={`${sheet.name}, ${sheet.cutouts.length} cut areas`} /></div><div class="card-info"><strong>{sheet.name}</strong><span>{sheet.material} · {quantity(sheet.thickness_mm, 'mm')} · {laserLabel(sheet.mode)}</span><small>{distance(sheet.bounds.max.x - sheet.bounds.min.x)} × {quantity(sheet.bounds.max.y - sheet.bounds.min.y, 'mm')} · {sheet.cutouts.length} cut areas</small><em>{onchoose && !compatible(sheet) ? 'Choose a matching material first' : sheet.used ? 'Used by a later cut' : sheet.state === 'remnant' ? 'Ready to nest' : sheet.state === 'completed' ? 'Ready to inspect' : 'Unfinished cut'}</em></div></button>{/each}</div>
  {#if loaded && !items.length}<div class="empty"><strong>{remnants ? 'No remaining sheets saved yet' : 'No cut sheets recorded yet'}</strong><p>{remnants ? 'Completed jobs appear in Cut history. Inspect one and save its remaining sheet here.' : 'Completed runs keep their sheet boundaries and cut areas. You can also mark a saved job as already cut.'}</p>{#if !onchoose && remnants}<button class="btn" onclick={() => { remnants = false; void load(); }}>View cut history</button>{/if}</div>{/if}
  {#if next}<button class="btn more" disabled={busy} onclick={() => load(true)}>Load older sheets</button>{/if}
  {#if busy && !items.length}<p>Loading sheets…</p>{/if}
</div>

{#if inspected}<RemnantInspector id={inspected} onclose={() => inspected = null} onchanged={() => { void load(); }} />{/if}
{#if adding}<Modal title="Mark a saved job as already cut" onclose={() => { if (!busy) adding = false; }}><p class="intro">Choose a job you have already cut. Its saved stock and part positions become a sheet record to inspect.</p><input class="job-search" type="search" bind:value={search} placeholder="Find a saved job…" aria-label="Find a cut job" /><div class="job-choices">{#each jobs as job}<button disabled={busy} onclick={() => report(job.id)}><strong>{job.name}</strong><span>{job.recipe.name} · {quantity(job.recipe.thickness_mm, 'mm')} · {laserLabel(job.recipe.laser)}</span></button>{/each}</div>{#if error}<p class="error">{error}</p>{/if}</Modal>{/if}

<style>
  .sheet-library { min-height:0; padding:20px; overflow-y:auto; flex:1; }
  .library-tools { display:flex; align-items:center; justify-content:space-between; gap:18px; margin-bottom:22px; }
  .library-tools button { min-height:48px; } .sheets-grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(220px,1fr)); gap:16px; }
  .sheet-card { overflow:hidden; display:flex; flex-direction:column; text-align:left; border:1px solid var(--line); border-radius:12px; color:var(--ink); background:var(--panel); cursor:pointer; padding:0; }
  .sheet-card:hover { border-color:var(--accent); } .sheet-card:disabled { opacity:.5; } .shape { height:158px; padding:10px; background:var(--panel-2); }
  .card-info { padding:16px; display:grid; gap:7px; } .card-info strong { font-size:14px; overflow-wrap:anywhere; } .card-info span { font-size:12px; color:var(--ink-2); } .card-info small { font-size:11px; color:var(--ink-3); } em { font-style:normal; font-size:11px; color:var(--accent); margin-top:3px; }
  .intro, .empty p { color:var(--ink-3); font-size:13px; line-height:1.65; } .empty { max-width:420px; padding:65px 20px; margin:auto; text-align:center; } .empty strong { font-size:18px; } .more { display:block; min-height:48px; margin:22px auto 0; } .error { color:var(--warn); }
  .job-search { width:100%; min-height:48px; padding:12px; border:1px solid var(--line); border-radius:8px; background:var(--panel-2); color:var(--ink); margin:12px 0; } .job-choices { display:grid; gap:8px; max-height:50vh; overflow:auto; } .job-choices button { min-height:72px; padding:16px; text-align:left; border:1px solid var(--line); border-radius:9px; background:var(--panel-2); color:var(--ink); cursor:pointer; } .job-choices span { display:block; font-size:12px; margin-top:7px; color:var(--ink-3); }
</style>
