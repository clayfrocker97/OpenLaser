<script lang="ts">
  // One plain status line. Tapping it shows why each control is unavailable,
  // the active alarms with how to clear them, and the original technical text.
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { setupAlarms } from '../lib/setup-alarms';
  import { alarmTitles, blockedControls, plain, type NamedGate, type Plain } from '../lib/plain';

  let { status, gates = [], tone = 'info' }: {
    /** The line itself: plain text, or a raw reason to put in plain words. */
    status: Plain | string;
    /** Controls whose unavailability the details explain. */
    gates?: NamedGate[];
    tone?: 'info' | 'ready' | 'warn';
  } = $props();

  let open = $state(false);
  const line = $derived(typeof status === 'string' ? plain(status, server.doc?.machine.alarms) : status);
  // A reason the line already gives is not repeated underneath it.
  const blocked = $derived(blockedControls(gates).filter((group) => group.reason.text !== line.text));
  const alarms = $derived((server.doc?.machine.alarms ?? []).filter((alarm) => alarm.blocking));
  const setup = $derived(setupAlarms(server.doc));
  const details = $derived([...new Set([line.detail, ...blocked.map((b) => b.reason.detail)].filter((d): d is string => !!d))]);
  const expandable = $derived(blocked.length > 0 || alarms.length > 0 || setup.length > 0 || details.length > 0);
  const shownTone = $derived(alarms.length + setup.length > 0 && tone !== 'ready' ? 'warn' : tone);
</script>

{#if line.text}
  <div class="status-line tone-{shownTone}">
    <button class="status-summary" onclick={() => (open = !open)} disabled={!expandable} aria-expanded={expandable ? open : undefined}>
      <span class="dot" aria-hidden="true"></span>
      <span class="status-text" role="status">{line.text}</span>
      {#if expandable}<span class="more">{open ? 'Hide' : 'Details'}</span>{/if}
    </button>
    {#if open && expandable}
      <div class="status-details">
        {#if alarms.length || setup.length}
          <ul class="status-alarms">
            {#each setup as alarm (alarm.title)}<li><strong>{alarm.title}</strong><span>{alarm.fix}</span></li>{/each}
            {#each alarmTitles(alarms) as title (title)}
              {@const alarm = alarms.find((row) => row.title === title)!}
              <li><strong>{title}</strong><span>{alarm.fix}</span></li>
            {/each}
          </ul>
          <button class="btn btn-ghost" onclick={() => (ui.modal = 'alarms')}>Open alarms</button>
        {/if}
        {#if blocked.length}
          <ul class="status-controls">
            {#each blocked as { names, reason } (names.join())}<li><strong>{names.join(', ')}</strong><span>{reason.text}</span></li>{/each}
          </ul>
        {/if}
        {#if details.length}
          <details class="technical"><summary>Technical details</summary>{#each details as detail}<code>{detail}</code>{/each}</details>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .status-line { display: grid; gap: 8px; min-width: 0; }
  .status-summary { display: flex; align-items: center; gap: 10px; min-height: 44px; width: 100%; padding: 0 12px; border: 1px solid var(--line); border-radius: 10px; background: var(--panel-2, transparent); color: var(--ink); font: inherit; font-size: var(--t-sm); text-align: left; cursor: pointer; }
  .status-summary:disabled { cursor: default; opacity: 1; }
  .dot { flex: none; width: 10px; height: 10px; border-radius: 50%; background: var(--ink-3); }
  .tone-ready .dot { background: var(--move); }
  .tone-warn .dot { background: var(--warn); }
  .tone-warn .status-summary { border-color: var(--warn); }
  .status-text { flex: 1; min-width: 0; font-weight: 600; overflow-wrap: anywhere; }
  .more { flex: none; color: var(--ink-3); font-size: var(--t-sm); }
  .status-details { display: grid; gap: 10px; padding: 12px; border: 1px solid var(--line); border-radius: 10px; font-size: var(--t-sm); }
  ul { display: grid; gap: 8px; margin: 0; padding: 0; list-style: none; }
  li { display: grid; gap: 2px; }
  li span { color: var(--ink-2); }
  .technical summary { cursor: pointer; color: var(--ink-3); font-size: var(--t-sm); min-height: 44px; display: flex; align-items: center; }
  .technical code { display: block; margin-top: 4px; font-size: var(--t-sm); color: var(--ink-3); overflow-wrap: anywhere; }
</style>
