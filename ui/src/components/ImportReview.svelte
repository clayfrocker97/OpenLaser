<script lang="ts">
  // The review of one drawing before it becomes a part: a preview with open
  // and self-crossing shapes and repairs marked, its size against the bed,
  // the layers to import and, for an SVG sized in pixels, the scale. Every
  // choice asks the server again, so the preview is what will be kept.
  // Nothing here moves the machine; skipping a file loses nothing.
  import Modal from './Modal.svelte';
  import type { ImportOptions, ImportReview, SvgScale } from '../api';
  import { boxOf, pathOf, viewBoxFor } from '../lib/svg';
  import { SCALES, contourKinds, layersChoosable, repairLines, scaleChoosable, toggleLayer } from '../lib/import-review';

  let { review, options, busy, error, remaining, onoptions, onimport, onskip, onstop }: {
    review: ImportReview;
    options: ImportOptions;
    busy: boolean;
    error: string;
    remaining: number;
    onoptions: (options: ImportOptions) => void;
    onimport: () => void;
    onskip: () => void;
    onstop: () => void;
  } = $props();

  const kinds = $derived(contourKinds(review));
  const box = $derived(boxOf(review.outline));
  const viewBox = $derived(box ? viewBoxFor(box, 4, 3) : '0 0 1 1');
  // Repair markers keep one size on screen whatever the drawing's size.
  const mark = $derived(box ? Math.max(box.maxX - box.minX, box.maxY - box.minY, 1e-6) * 0.015 : 1);
  const repairs = $derived(repairLines(review.repairs));
  const scale = $derived<SvgScale | null>(review.scale?.scale ?? null);
  const size = $derived(review.size);
  const digits = $derived(size && Math.max(size.size[0], size.size[1]) >= 100 ? 1 : 2);

  function chooseScale(value: SvgScale): void {
    if (busy || value === scale) return;
    onoptions({ ...options, scale: value });
  }

  function chooseLayer(name: string, on: boolean): void {
    if (busy) return;
    onoptions({ ...options, layers: toggleLayer(review.layers, name, on) });
  }
</script>

<Modal title={`Review ${review.name}`} wide dismissable={!busy} onclose={onskip}>
  <div class="import-review">
    <figure class="preview" aria-label="Preview">
      <svg viewBox={viewBox} preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${review.contours} shapes`}>
        {#each review.outline as line, i (i)}
          <path d={pathOf(line)} class={kinds[i]} vector-effect="non-scaling-stroke" />
        {/each}
        {#each review.repairs.gaps as gap, i (i)}
          <circle class="gap" cx={gap.at.x} cy={-gap.at.y} r={mark} vector-effect="non-scaling-stroke" />
        {/each}
        {#each review.repairs.duplicates as duplicate, i (i)}
          <rect class="duplicate" x={duplicate.at.x - mark * 0.7} y={-duplicate.at.y - mark * 0.7} width={mark * 1.4} height={mark * 1.4} vector-effect="non-scaling-stroke" />
        {/each}
      </svg>
      <figcaption>
        <span class="key closed">Closed {review.contours - review.open.length - review.crossing.length}</span>
        {#if review.open.length}<span class="key open">Open {review.open.length}</span>{/if}
        {#if review.crossing.length}<span class="key crossing">Crosses itself {review.crossing.length}</span>{/if}
        {#if review.repairs.gaps.length}<span class="key gap">Gap closed</span>{/if}
        {#if review.repairs.duplicates.length}<span class="key duplicate">Repeat removed</span>{/if}
      </figcaption>
    </figure>

    <div class="facts">
      {#if size}
        <p class="size" class:warn={size.tiny || size.exceeds_bed}>
          <strong>{size.size[0].toFixed(digits)} × {size.size[1].toFixed(digits)} mm</strong>
          {#if size.bed}<span>Bed {size.bed[0].toFixed(0)} × {size.bed[1].toFixed(0)} mm</span>{/if}
          {#if size.exceeds_bed}<span class="flag">Larger than the bed</span>{/if}
          {#if size.tiny}<span class="flag">Very small; check the units</span>{/if}
        </p>
      {/if}

      {#if scaleChoosable(review)}
        <fieldset class="choices">
          <legend>{review.scale?.units === 'ambiguous' ? 'The file does not say how large a pixel is' : review.scale?.units === 'illustrator' ? 'Written by Illustrator: 72 px per inch' : 'Written by Inkscape: 96 px per inch'}</legend>
          <div class="scales" role="group" aria-label="Scale">
            {#each SCALES as [value, label] (value)}
              <button aria-pressed={scale === value} disabled={busy} onclick={() => chooseScale(value)}>{label}</button>
            {/each}
          </div>
        </fieldset>
      {/if}

      {#if layersChoosable(review)}
        <fieldset class="choices">
          <legend>Layers</legend>
          <ul class="import-layers">
            {#each review.layers as layer (layer.name)}
              <li>
                <label>
                  <input type="checkbox" checked={layer.imported} disabled={busy} onchange={(e) => chooseLayer(layer.name, e.currentTarget.checked)} />
                  <span class="name">{layer.name}</span>
                  <small>{layer.entities} {layer.entities === 1 ? 'entity' : 'entities'}{layer.hidden ? ' · off in the file' : ''}</small>
                </label>
              </li>
            {/each}
          </ul>
        </fieldset>
      {/if}

      {#if repairs.length}
        <div class="repairs"><h3>Repaired</h3><ul>{#each repairs as line (line)}<li>{line}</li>{/each}</ul></div>
      {/if}

      {#if review.warnings.length}
        <ul class="warnings">{#each review.warnings as warning (warning)}<li>{warning}</li>{/each}</ul>
      {/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
    </div>
  </div>

  <div class="actions">
    {#if remaining > 0}<button class="btn btn-ghost lg" disabled={busy} onclick={onstop}>Stop importing</button>{/if}
    <button class="btn btn-ghost lg" disabled={busy} onclick={onskip}>Skip file</button>
    <button class="btn btn-primary lg" disabled={busy || review.contours === 0} onclick={onimport}>{busy ? 'Checking…' : 'Import'}</button>
  </div>
  {#if review.contours === 0}<p class="gate-reason">Choose at least one layer with geometry.</p>{/if}
</Modal>

<style>
  .import-review { display: grid; grid-template-columns: minmax(0, 3fr) minmax(0, 2fr); gap: 16px; }
  .preview { margin: 0; display: grid; gap: 8px; align-content: start; }
  .preview svg { width: 100%; aspect-ratio: 4 / 3; background: var(--panel-2); border: 1px solid var(--line); border-radius: 10px; }
  .preview path { fill: none; stroke: var(--ink); stroke-width: 1.2; }
  .preview path.open { stroke: var(--hold); stroke-width: 2; stroke-dasharray: 6 3; }
  .preview path.crossing { stroke: var(--danger); stroke-width: 2.4; }
  .preview .gap { fill: none; stroke: var(--accent); stroke-width: 2; }
  .preview .duplicate { fill: none; stroke: var(--move); stroke-width: 2; }
  figcaption { display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: var(--t-sm); color: var(--ink-2); }
  .key::before { content: ''; display: inline-block; width: 14px; height: 0; margin-right: 6px; vertical-align: middle; border-top: 2px solid var(--ink); }
  .key.open::before { border-top: 2px dashed var(--hold); }
  .key.crossing::before { border-top-color: var(--danger); }
  .key.gap::before { width: 8px; height: 8px; border: 2px solid var(--accent); border-radius: 50%; }
  .key.duplicate::before { width: 8px; height: 8px; border: 2px solid var(--move); }
  .facts { display: grid; gap: 14px; align-content: start; min-width: 0; }
  .size { margin: 0; display: flex; flex-wrap: wrap; gap: 4px 10px; align-items: baseline; }
  .size span { color: var(--ink-3); font-size: var(--t-sm); }
  .size .flag { color: var(--hold); font-weight: 600; }
  .choices { margin: 0; padding: 0; border: 0; display: grid; gap: 8px; }
  .choices legend { padding: 0; margin-bottom: 8px; color: var(--ink-2); font-size: var(--t-sm); }
  .scales { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px; }
  .scales button { min-height: var(--touch); border: 1px solid var(--line); border-radius: 10px; background: var(--panel-2); color: var(--ink); cursor: pointer; touch-action: manipulation; }
  .scales button[aria-pressed="true"] { border-color: var(--accent); color: var(--accent); background: var(--accent-soft); }
  .import-layers { list-style: none; margin: 0; padding: 0; display: grid; gap: 8px; max-height: 240px; overflow: auto; }
  .import-layers label { min-height: 44px; display: flex; align-items: center; gap: 10px; padding: 0 10px; border: 1px solid var(--line); border-radius: 10px; cursor: pointer; touch-action: manipulation; }
  .import-layers input { width: 22px; height: 22px; flex: none; }
  .import-layers .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .import-layers small { margin-left: auto; color: var(--ink-3); white-space: nowrap; }
  .repairs h3 { margin: 0 0 4px; font-size: var(--t-base); }
  .repairs ul, .warnings { margin: 0; padding-left: 18px; display: grid; gap: 4px; line-height: 1.5; }
  .warnings { color: var(--ink-2); font-size: var(--t-sm); }
  .error { color: var(--warn); margin: 0; }
  .actions { display: flex; justify-content: flex-end; flex-wrap: wrap; gap: 8px; margin-top: 16px; }
  @media (max-width: 720px) {
    .import-review { grid-template-columns: minmax(0, 1fr); }
  }
</style>
