<script lang="ts">
  import { displayNumber, quantity, unitLabel } from '../lib/units.svelte';
  import Modal from './Modal.svelte';
  import FontPicker from './FontPicker.svelte';
  import { api, type TextOptions, type TextPreview } from '../api/client';
  import { ui } from '../stores/ui.svelte';
  import { server } from '../stores/server.svelte';
  import { osk } from '../lib/osk.svelte';
  import { explain } from '../lib/format';
  import { boxOf, pathOf, viewBoxFor } from '../lib/svg';

  let { onclose }: { onclose: () => void } = $props();
  let value = $state('OpenLaser');
  let family = $state<TextOptions['family']>('sans');
  let font = $state<string | undefined>();
  let importing = $state(false);
  let fontRevision = $state(0);
  let size = $state(20);
  let bold = $state(false);
  let alignment = $state<TextOptions['alignment']>('left');
  let preview = $state<TextPreview | null>(null);
  let pending = $state(false);
  let creating = $state(false);
  let error = $state('');
  const options = $derived<TextOptions>({ value, family, size, bold, alignment, font });
  const box = $derived(preview ? boxOf(preview.outline) : null);

  $effect(() => {
    const next = options;
    void fontRevision;
    let cancelled = false;
    preview = null;
    error = '';
    const hasText = !!next.value.trim();
    pending = hasText;
    if (!hasText) return;
    const timer = setTimeout(async () => {
      try { const result = await api.previewText(next); if (!cancelled) preview = result; }
      catch (reason) { if (!cancelled) error = explain(reason); }
      finally { if (!cancelled) pending = false; }
    }, 180);
    return () => { cancelled = true; clearTimeout(timer); };
  });

  function close(): void { if (!creating && !importing) { osk.close(); onclose(); } }

  async function create(): Promise<void> {
    if (creating || importing || pending || !preview) return;
    creating = true;
    error = '';
    osk.close();
    try {
      const name = value.trim().split(/\r?\n/)[0]!.replace(/[\\/]/g, ' ').slice(0, 80);
      const { id } = await api.createText(name || 'Text', options);
      ui.selected = id;
      // With a job open the text joins it beside the sheet; otherwise it opens as the job.
      if (server.doc?.draft) {
        await api.addParts([id]);
        ui.tab = 'setup';
        onclose();
        ui.say('Text added beside the sheet · Undo takes it off');
      } else {
        await api.openPart(id);
        ui.tab = 'setup';
        onclose();
        ui.say('Text added. Choose a material to prepare it for cutting.');
      }
    } catch (reason) { error = explain(reason); }
    finally { creating = false; }
  }
</script>

<Modal title="Add text" wide onclose={close}>
  <div class="text-editor">
    <label class="text-label">Text<textarea aria-label="Text to cut" rows="4" bind:value disabled={creating} placeholder="Type or paste your text…"></textarea></label>
    <div class="text-controls">
      <div class="text-fonts"><FontPicker bind:family bind:font bind:importing disabled={creating} onimport={() => fontRevision++} /></div>
      <div class="text-size-row"><span>Font size</span><button class="text-number" data-numpad aria-label="Font size" disabled={creating} onclick={() => osk.number('Font size', size, 'mm', (next) => (size = next))}>{displayNumber(size, 'mm')} <small>{unitLabel('mm')}</small></button></div>
      {#if !font}<div class="text-choices text-weight">
        <button class:on={!bold} aria-pressed={!bold} disabled={creating} onclick={() => (bold = false)}>Regular</button>
        <button class:on={bold} aria-pressed={bold} disabled={creating} onclick={() => (bold = true)}>Bold</button>
      </div>{:else}<p class="muted text-face-note">Weight and style come from the selected font.</p>{/if}
      <div><div class="text-label">Align lines</div><div class="text-choices">
        {#each ['left', 'center', 'right'] as side}
          <button class:on={alignment === side} aria-pressed={alignment === side} disabled={creating} onclick={() => (alignment = side as TextOptions['alignment'])}>{side[0]!.toUpperCase() + side.slice(1)}</button>
        {/each}
      </div></div>
    </div>
    <div class="text-preview" aria-label="Text outline preview" aria-busy={pending}>
      {#if preview && box}
        <svg viewBox={viewBoxFor(box, 600, 190, 0.1)} role="img" aria-label="Cuttable text outlines"><path d={preview.outline.map((line) => pathOf(line)).join(' ')} fill-rule="evenodd" vector-effect="non-scaling-stroke" /></svg>
        <div class="text-dimensions">{displayNumber(preview.width, 'mm', 1)} × {quantity(preview.height, 'mm', 1)} · {preview.contours} outlines</div>
      {:else}<span class="muted">{pending ? 'Preparing text…' : 'Your text preview appears here.'}</span>{/if}
    </div>
    {#if error}<p class="warn-text" role="alert">{error}</p>{/if}
    {#each preview?.warnings ?? [] as warning}<p class="warn-text" role="status">{warning}</p>{/each}
    <div class="text-footer"><span class="muted">Overlapping letters are welded into cutting outlines.</span><button class="btn btn-primary" disabled={creating || importing || pending || !preview} onclick={create}>{creating ? 'Adding…' : 'Add text'}</button></div>
  </div>
</Modal>

<style>
  .text-editor { display: grid; gap: 18px; }
  .text-label { display: grid; gap: 8px; font-size: var(--t-sm); color: var(--ink-2); }
  textarea { width: 100%; resize: vertical; min-height: 100px; border: 1px solid var(--line); border-radius: 10px; background: var(--panel-2); color: var(--ink); padding: 12px; font: inherit; font-size: var(--t-lg); line-height: 1.4; }
  .text-controls { display: grid; grid-template-columns: 1.3fr 1fr; gap: 14px 24px; }
  .text-fonts { grid-column: 1 / -1; }
  .text-face-note { align-self: center; font-size: var(--t-sm); margin: 0; }
  .text-choices { display: flex; gap: 6px; }
  .text-choices button, .text-number { min-height: 44px; border: 1px solid var(--line); border-radius: 8px; background: var(--panel-2); color: var(--ink); font: inherit; cursor: pointer; padding: 10px 15px; }
  .text-choices button { flex: 1; }
  .text-choices .on { color: var(--accent); border-color: var(--accent); background: color-mix(in srgb, var(--accent) 9%, var(--panel)); }
  .text-size-row { display: flex; align-items: end; justify-content: space-between; gap: 10px; padding-bottom: 1px; font-size: var(--t-sm); }
  .text-size-row > span { align-self: center; }
  .text-weight { align-self: end; }
  .text-preview { min-height: 220px; display: grid; place-items: center; gap: 10px; padding: 16px; border: 1px solid var(--line); border-radius: 12px; background: var(--panel-2); }
  .text-preview svg { width: 100%; height: 190px; color: var(--accent); }
  .text-preview path { fill: color-mix(in srgb, var(--accent) 14%, transparent); stroke: currentColor; stroke-width: 1; }
  .text-dimensions { font-size: var(--t-sm); color: var(--ink-2); }
  .text-footer { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
  .text-footer span { font-size: var(--t-sm); }
  @media (max-width: 650px) { .text-controls { grid-template-columns: 1fr; } .text-footer { flex-wrap: wrap; } }
</style>
