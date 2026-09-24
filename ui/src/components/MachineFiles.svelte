<script lang="ts">
  import { settingsEdits } from '../lib/settings-edits.svelte';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain } from '../lib/format';
  let { kind = 'backup', disabled = false }: { kind?: 'backup' | 'soft'; disabled?: boolean } = $props();
  let picker = $state<HTMLInputElement>();
  const doc = $derived(server.doc!);
  // The vendor's backup; importing one replaces the last and binds the
  // machine to it.
  async function importBackup(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    try { await settingsEdits.file('backup', file, doc.files.backup?.sha256 ?? ''); ui.modal = 'pending'; } catch (error) { ui.say(explain(error), true); }
  }
  async function importSoft(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    try { await settingsEdits.file('soft', file, doc.soft.sha256 ?? ''); ui.modal = 'pending'; } catch (error) { ui.say(explain(error), true); }
  }
</script>
{#if kind === 'backup'}
  <button class="btn btn-ghost" {disabled} onclick={() => picker?.click()}>Import backup.xml</button>
  <input bind:this={picker} type="file" accept=".xml" hidden {disabled} onchange={importBackup} />
{:else}
  <div class="setting-group"><h3>Process settings</h3>
    <div class="setting"><div class="lbl">Process INI<small>{doc.soft.name ?? 'Default process timing'}</small></div><button
        class="btn btn-ghost" {disabled} onclick={() => picker?.click()}>Import INI</button><input
        bind:this={picker} type="file" accept=".ini" hidden {disabled} onchange={importSoft} /></div>
    <details class="process-details"><summary>Details</summary><div
        class="setting"><div class="lbl">Follow / retract</div><span class="val">{doc.soft.follow_ms} ms</span></div><div
        class="setting"><div class="lbl">Pierce-height move</div><span class="val">{doc.soft.section_drill_ms} ms</span></div><div
        class="setting"><div class="lbl">Analog minimum</div><span class="val">{doc.soft.analog_minimum}</span></div>{#if doc.soft.name}<a
        class="btn btn-ghost" href="/api/machine/soft" download>Download original INI</a>{/if}</details>
  </div>
{/if}
<style>
.process-details summary { cursor: pointer; color: var(--ink-2); font-size: var(--t-sm); }
</style>
