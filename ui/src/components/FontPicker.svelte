<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type FontFace, type TextOptions } from '../api/client';
  import { explain } from '../lib/format';
  import { osk } from '../lib/osk.svelte';

  let { family = $bindable('sans'), font = $bindable(), importing = $bindable(false), disabled = false, onimport }: {
    family: TextOptions['family']; font?: string; importing?: boolean; disabled?: boolean; onimport: () => void;
  } = $props();
  let fonts = $state<FontFace[]>([]);
  let filter = $state('');
  let notices = $state<string[]>([]);
  let errors = $state<string[]>([]);
  let fileInput: HTMLInputElement;
  const visible = $derived(fonts.filter(face => `${face.family} ${face.name}`.toLowerCase().includes(filter.toLowerCase())));
  const busy = $derived(disabled || importing);

  onMount(() => {
    let cancelled = false;
    api.fonts().then(result => { if (!cancelled) {
      const merged = new Map(result.fonts.map(face => [face.id, face]));
      for (const face of fonts) merged.set(face.id, face);
      fonts = [...merged.values()];
    } })
      .catch(reason => { if (!cancelled) errors = [explain(reason)]; });
    return () => { cancelled = true; };
  });

  function style(face: FontFace): string {
    const weight = ({ 100: 'Thin', 200: 'Extra light', 300: 'Light', 400: 'Regular', 500: 'Medium', 600: 'Semibold', 700: 'Bold', 800: 'Extra bold', 900: 'Black' } as Record<number, string>)[face.weight] ?? `Weight ${face.weight}`;
    return [weight, face.style === 'normal' ? '' : face.style, face.stretch === 'normal' ? '' : face.stretch].filter(Boolean).join(' · ');
  }

  async function importFiles(): Promise<void> {
    const files = [...(fileInput.files ?? [])];
    fileInput.value = '';
    if (!files.length || busy) return;
    importing = true;
    errors = []; notices = [];
    let selected: string | undefined;
    for (const file of files) {
      try {
        if (file.size > 64 * 1024 * 1024) throw new Error('The font exceeds 64 MiB.');
        const result = await api.importFont(file.name, await file.arrayBuffer());
        const merged = new Map(fonts.map(face => [face.id, face]));
        for (const face of result.fonts) merged.set(face.id, face);
        fonts = [...merged.values()].sort((a, b) => a.family.localeCompare(b.family) || a.name.localeCompare(b.name));
        selected ??= result.fonts[0]?.id;
        notices.push(`${file.name}: ${result.existing ? 'already imported' : 'imported'}.`);
      } catch (reason) { errors.push(`${file.name}: ${explain(reason)}`); }
    }
    if (selected) { font = selected; filter = ''; }
    importing = false;
    onimport();
  }
</script>

<div class="font-picker">
  <div class="font-heading"><span>Font</span><button class="font-import" disabled={busy} onclick={() => { osk.close(); fileInput.click(); }}>{importing ? 'Importing…' : '+ Import fonts'}</button></div>
  <input bind:this={fileInput} type="file" accept=".ttf,.otf,.ttc,.otc" multiple aria-label="Import font files" onchange={importFiles} hidden />
  <div class="font-defaults">
    {#each [['sans', 'Sans'], ['serif', 'Serif'], ['mono', 'Mono']] as [id, label]}
      <button class:on={!font && family === id} aria-pressed={!font && family === id} disabled={busy} onclick={() => { family = id as TextOptions['family']; font = undefined; }}>{label}</button>
    {/each}
  </div>
  {#if fonts.length}
    <div class="font-imported-heading"><span>Imported fonts</span><small>{fonts.length}</small></div>
    {#if fonts.length > 8}<input class="font-search" aria-label="Search imported fonts" placeholder="Search fonts…" bind:value={filter} />{/if}
    <div class="font-list" aria-label="Imported fonts">
      {#each visible as face (face.id)}
        <button class:on={font === face.id} aria-pressed={font === face.id} title={face.name} disabled={busy} onclick={() => font = face.id}><span>{face.family}</span><small>{style(face)}</small></button>
      {/each}
      {#if !visible.length}<span class="muted">No matching fonts.</span>{/if}
    </div>
  {/if}
  <p class="font-help">TTF, OTF or font collections · Saved in OpenLaser for text and SVG imports.</p>
  {#if notices.length}<div class="font-notices" role="status">{#each notices as notice}<p>{notice}</p>{/each}</div>{/if}
  {#each errors as error}<p class="warn-text" role="alert">{error}</p>{/each}
</div>

<style>
  .font-picker { display: grid; gap: 8px; min-width: 0; }
  .font-heading, .font-imported-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; font-size: 13px; color: var(--ink-2); }
  .font-heading button { color: var(--accent); min-height: 40px; padding: 8px 12px; }
  button { min-height: 44px; border: 1px solid var(--line); border-radius: 8px; background: var(--panel-2); color: var(--ink); font: inherit; cursor: pointer; padding: 10px 15px; }
  button.on { color: var(--accent); border-color: var(--accent); background: color-mix(in srgb, var(--accent) 9%, var(--panel)); }
  .font-defaults { display: flex; gap: 6px; }
  .font-defaults button { flex: 1; }
  .font-imported-heading { margin-top: 6px; }
  .font-list { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px; max-height: 170px; overflow: auto; }
  .font-list button { display: grid; gap: 5px; text-align: left; overflow-wrap: anywhere; }
  .font-list small, .font-help { color: var(--ink-2); font-size: 11px; }
  .font-help, .font-notices p, .warn-text { margin: 0; }
  .font-notices { color: var(--ink-2); font-size: 12px; overflow-wrap: anywhere; max-height: 72px; overflow: auto; }
  .font-search { min-height: 44px; width: 100%; border: 1px solid var(--line); border-radius: 8px; padding: 10px 12px; color: var(--ink); background: var(--panel-2); font: inherit; }
  @media (max-width: 420px) { .font-list { grid-template-columns: 1fr; } }
</style>
