<script lang="ts">
  import { quantity } from '../lib/units.svelte';
  import { onMount } from 'svelte';
  import Modal from './Modal.svelte';
  import SheetShape from './SheetShape.svelte';
  import { api } from '../api/client';
  import type { SheetView } from '../api';
  import { osk } from '../lib/osk.svelte';
  import { explain, laserLabel } from '../lib/format';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  let { id, onclose, onchanged = () => undefined }: { id: string; onclose: () => void; onchanged?: () => void } = $props();
  let sheet = $state<SheetView | null>(null);
  let name = $state('');
  let x = $state(0), y = $state(0), width = $state(0), height = $state(0);
  let busy = $state(false), error = $state('');
  const status = $derived(
    !sheet ? ''
    : sheet.used ? 'Used by a later job'
    : sheet.state === 'remnant' ? 'Ready to nest'
    : sheet.reported ? 'Marked cut by operator'
    : sheet.state === 'completed' ? 'Job completed'
    : 'Unfinished cut');
  const dimensions: [string, number][] = $derived([['Width', width], ['Height', height], ['Front-left X', x], ['Front-left Y', y]]);
  onMount(() => {
    void api.sheet(id).then(value => {
      sheet = value;
      name = value.state === 'remnant' ? value.name : `${value.name} remnant`;
      x = value.bounds.min.x;
      y = value.bounds.min.y;
      width = value.bounds.max.x - x;
      height = value.bounds.max.y - y;
    }).catch(e => error = explain(e));
  });
  function close(): void {
    if (!busy) onclose();
  }
  function editDimension(label: string, value: number): void {
    osk.number(label, value, 'mm', v => {
      if (label === 'Width') width = v;
      else if (label === 'Height') height = v;
      else if (label === 'Front-left X') x = v;
      else y = v;
    });
  }
  async function run(action: () => Promise<void>): Promise<void> {
    if (busy) return; busy = true; error = '';
    try { await action(); } catch (e) { error = explain(e); } finally { busy = false; }
  }
  async function save(): Promise<void> {
    if (!sheet) return;
    await api.saveRemnant(id, {
      revision: sheet.revision,
      name,
      bounds: sheet.boundary_known ? null : { min: { x, y }, max: { x: x + width, y: y + height } },
    });
    ui.say('Remaining sheet saved in Remnants.');
    onchanged();
    onclose();
  }
  async function use(): Promise<void> {
    await api.setStock({ kind: 'remnant', id });
    ui.tab = 'setup';
    ui.setupPanel = 'nest';
    onclose();
  }
</script>

<Modal title={sheet?.state === 'remnant' ? 'Remaining sheet' : 'Save remaining sheet'} wide onclose={close}>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if sheet}
    <div class="remnant-review">
      <div class="shape">
        <SheetShape outline={sheet.outline} cutouts={sheet.cutouts} />
        <div class="shape-key">
          <span><i></i>Remaining material</span>
          <span><i class="removed"></i>Cut areas kept clear</span>
        </div>
      </div>
      <div class="details">
        <span class="eyebrow">{status}</span>
        <h3>{sheet.name}</h3>
        <p>{sheet.material} · {quantity(sheet.thickness_mm, 'mm')} · {laserLabel(sheet.mode)}</p>
        <p>{sheet.cutouts.length} cut areas will be excluded from future nests. Check the remaining sheet against this drawing before saving it.</p>
        {#if sheet.clearance > 0}
          <p>Previous kerf and leads keep at least {quantity(sheet.clearance, 'mm')} clear around cut areas.</p>
        {/if}
        {#if sheet.state === 'completed' && !sheet.used}
          <button class="field" disabled={busy} onclick={() => osk.text('Remnant name', name, v => name = v)}>
            <span>Name</span>
            <strong>{name}</strong>
          </button>
          {#if !sheet.boundary_known}
            <p class="missing">This job had no sheet boundary. Enter the actual sheet size and its front-left corner in the drawing shown.</p>
            <div class="dimensions">
              {#each dimensions as [label, value]}
                <button class="field" disabled={busy} onclick={() => editDimension(label, value)}>
                  <span>{label}</span>
                  <strong>{quantity(value, 'mm')}</strong>
                </button>
              {/each}
            </div>
          {/if}
        {/if}
        {#if sheet.state === 'interrupted' || sheet.state === 'cutting'}
          <p class="missing">This record has no completed cut. Resume the job to finish it; planned part areas stay reserved.</p>
        {/if}
        <div class="primary">
          {#if sheet.state === 'completed' && !sheet.used}
            <button
              class="btn btn-primary lg block"
              disabled={busy || !name.trim() || width <= 0 || height <= 0}
              onclick={() => run(save)}>Save inspected remnant</button>
          {:else if sheet.state === 'remnant' && !sheet.used}
            <button class="btn btn-primary lg block" disabled={busy || !server.doc?.draft} onclick={() => run(use)}>
              Use as nesting stock
            </button>
          {:else}
            <button class="btn lg block" onclick={onclose}>Done</button>
          {/if}
        </div>
      </div>
    </div>
  {:else if !error}<p>Loading sheet…</p>{/if}
</Modal>

<style>
  .remnant-review { display:grid; grid-template-columns:minmax(0,1.3fr) minmax(260px,1fr); gap:30px; }
  .shape { min-height:300px; max-height:440px; padding:14px; background:var(--panel-2); border:1px solid var(--line); border-radius:12px; display:flex; flex-direction:column; }
  .shape :global(svg) { flex:1; }
  .shape-key { display:flex; gap:18px; flex-wrap:wrap; font-size:var(--t-sm); color:var(--ink-3); padding-top:14px; }
  .shape-key span { display:flex; align-items:center; gap:6px; }
  i { width:12px; height:12px; background:var(--accent-soft); border:1px solid var(--ink-3); }
  i.removed { background:var(--panel-2); }
  .eyebrow { font-size:var(--t-sm); color:var(--accent); text-transform:uppercase; letter-spacing:.1em; }
  h3 { font-size:var(--t-xl); text-transform:none; color:var(--ink); line-height:1.2; letter-spacing:0; margin:10px 0 12px; }
  p { font-size:var(--t-sm); line-height:1.65; color:var(--ink-3); margin:0 0 18px; }
  .error { color:var(--warn); }
  .field { display:block; width:100%; padding:14px 16px; min-height:68px; border:1px solid var(--line); border-radius:10px; text-align:left; background:var(--panel-2); color:var(--ink); cursor:pointer; margin-bottom:12px; }
  .field span { display:block; color:var(--ink-3); font-size:var(--t-sm); margin-bottom:6px; }
  .field strong { font-size:var(--t-base); overflow-wrap:anywhere; }
  .dimensions { display:grid; grid-template-columns:1fr 1fr; gap:10px; }
  .dimensions .field { margin:0; }
  .missing { padding:12px; border-left:3px solid var(--accent); background:var(--accent-soft); font-size:var(--t-sm); }
  .primary { margin-top:25px; }
  .primary button { min-height:56px; }
  @media(max-width:760px) { .remnant-review { grid-template-columns:1fr; } .shape { min-height:220px; max-height:300px; } }
</style>
