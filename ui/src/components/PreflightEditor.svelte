<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from './Modal.svelte';
  import CheckEditor from './CheckEditor.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { settingsEdits, withPostflight } from '../lib/settings-edits.svelte';
  import { explain, laserLabel, plural } from '../lib/format';
  import type { Check, JobPreflight, LaserMode, PreflightPreferences } from '../api';

  let { scope, onclose }: { scope: 'defaults' | 'job' | 'pause' | 'postflight'; onclose: () => void } = $props();
  let preferences = $state<PreflightPreferences | null>(null);
  let base: PreflightPreferences | null = null;
  let mode = $state<LaserMode>('fiber');
  let choice = $state<JobPreflight['kind']>('inherit');
  let custom = $state<Check[]>([]);
  let revision = 0;
  let saving = $state(false);
  let error = $state('');

  onMount(() => {
    mode = server.doc?.draft?.recipe?.laser ?? server.doc?.mode ?? 'fiber';
    if (scope === 'job' && server.doc?.draft) {
      const draft = server.doc.draft;
      revision = draft.revision;
      choice = draft.preflight.kind;
      if (draft.preflight.kind === 'custom') custom = structuredClone($state.snapshot(draft.preflight.steps));
    }
    Promise.all([api.preflightPreferences(), settingsEdits.ready]).then(([value]) => {
      base = value;
      preferences = scope !== 'job' && settingsEdits.entries.preflight ? withPostflight(settingsEdits.entries.preflight.value, value) : value;
    }).catch((e) => (error = explain(e)));
  });

  function close(): void {
    onclose();
  }

  async function choose(kind: JobPreflight['kind']): Promise<void> {
    if (kind === 'off' && choice !== 'off' && !await ui.confirm({ title: 'Turn this job’s checklist off?', body: 'Start will run this job without asking for any preflight checks.', confirm: 'Turn checklist off', danger: true })) return;
    if (kind === 'custom' && !custom.length && preferences) custom = structuredClone($state.snapshot(preferences[mode].steps));
    choice = kind;
  }

  async function save(): Promise<void> {
    if (!preferences || saving) return;
    saving = true;
    error = '';
    try {
      if (scope !== 'job' && base) { await settingsEdits.preflight(base, $state.snapshot(preferences)); ui.modal = 'pending'; }
      else await api.setPreflight(choice === 'custom' ? { kind: 'custom', steps: $state.snapshot(custom) } : { kind: choice }, revision);
      if (scope === 'job') ui.say('Checklist set · save the job');
      onclose();
    } catch (e) { error = explain(e); } finally { saving = false; }
  }
</script>

<Modal title={scope === 'postflight' ? 'Postflight defaults' : scope === 'pause' ? 'Pause defaults' : scope === 'defaults' ? 'Preflight defaults' : 'Job preflight'} onclose={close}>
  {#if preferences}
    {#if scope !== 'job'}
      <div class="seg block"><button class:on={mode === 'fiber'} onclick={() => (mode = 'fiber')}>Fiber</button><button class:on={mode === 'co2'} onclick={() => (mode = 'co2')}>CO₂</button></div>
      {#if scope === 'pause'}
        <label class="preflight-toggle"><input type="checkbox" bind:checked={preferences.pause[mode].enabled}>Show when paused and before Resume</label>
        <CheckEditor phase="pause" bind:steps={preferences.pause[mode].steps} />
      {:else if scope === 'postflight'}
        <label class="preflight-toggle"><input type="checkbox" bind:checked={preferences.postflight[mode].enabled}>Show after job completion</label>
        <CheckEditor phase="postflight" bind:steps={preferences.postflight[mode].steps} />
      {:else}
        <label class="preflight-toggle"><input type="checkbox" bind:checked={preferences[mode].enabled}>Show before Start</label>
        <CheckEditor bind:steps={preferences[mode].steps} />
      {/if}
    {:else}
      <div class="seg block"><button class:on={choice === 'inherit'} onclick={() => choose('inherit')}>Use defaults</button><button class:on={choice === 'custom'} onclick={() => choose('custom')}>Custom</button><button class:on={choice === 'off'} onclick={() => choose('off')}>Off</button></div>
      {#if choice === 'custom'}
        <CheckEditor bind:steps={custom} />
      {:else if choice === 'inherit'}
        <p class="muted">{laserLabel(mode)} defaults · {preferences[mode].enabled ? plural(preferences[mode].steps.length, 'check') : 'checklist off'}</p>
        {#if preferences[mode].enabled}<ul class="inherited">{#each preferences[mode].steps as step}<li>{step.text}</li>{/each}</ul>{/if}
      {:else}
        <p class="muted">This job's checklist is off.</p>
      {/if}
    {/if}
    <div class="editor-footer"><button class="btn btn-ghost" onclick={close}>Close</button><button class="btn btn-primary" disabled={saving} onclick={save}>{saving ? 'Saving…' : scope !== 'job' ? 'Review changes' : 'Save'}</button></div>
  {:else if !error}<p class="muted">Loading checklist…</p>{/if}
  {#if error}<p class="warn-text" role="alert">{error}</p>{/if}
</Modal>

<style>
  .preflight-toggle { display: flex; align-items: center; gap: 12px; padding: 8px 0; cursor: pointer; }
  input { accent-color: var(--start); width: 20px; height: 20px; flex: 0 0 20px; }
  .editor-footer { display: flex; justify-content: flex-end; gap: 10px; padding-top: 12px; border-top: 1px solid var(--line); }
  .inherited { margin: 0; padding-left: 22px; color: var(--ink-2); line-height: 1.8; }
  p { margin: 0; }
</style>
