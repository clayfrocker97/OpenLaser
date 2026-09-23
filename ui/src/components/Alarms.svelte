<script lang="ts">
  import { untrack } from 'svelte';
  import Modal from './Modal.svelte';
  import HoldButton from './HoldButton.svelte';
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain } from '../lib/format';
  import { alarmTitles, groupAlarms } from '../lib/plain';
  import type { AlarmSession, HistoryPage } from '../api';

  let { onclose }: { onclose: () => void } = $props();
  const alarms = $derived(server.doc?.machine.alarms ?? []);
  const faulted = $derived(server.doc?.machine.connection.state === 'faulted' ? server.doc.machine.connection.reason : null);
  const historyRevision = $derived(server.doc?.alarm_history.revision ?? 0);
  const recordingError = $derived(server.doc?.alarm_history.error);
  let page = $state<HistoryPage | null>(null);
  let error = $state('');
  let loading = $state(false);
  let resetting = $state(false);
  let serial = 0;
  const time = (seconds: number) => new Date(seconds * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const date = (seconds: number) => new Date(seconds * 1000).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  const keyOf = (source: string, id: number | null) => `${source}:${id === null ? 'None' : `Some(${id})`}`;
  const pastRows = (session: AlarmSession) => session.alarms.filter((row) => session.id !== page?.current || (row.key === 'connection' ? !faulted : !alarms.some((alarm) => keyOf(alarm.source, alarm.id) === row.key))).sort((a, b) => b.last - a.last);

  async function load(older = false): Promise<void> {
    const id = ++serial;
    loading = true;
    error = '';
    try {
      const next = await api.alarmHistory(older ? page?.before ?? undefined : undefined);
      if (id !== serial) return;
      if (older && page) {
        page = { ...next, sessions: [...page.sessions, ...next.sessions.filter((s) => !page?.sessions.some((old) => old.id === s.id))] };
      } else {
        const retained = page?.sessions.filter((s) => s.id < (next.sessions.at(-1)?.id ?? '') && !next.sessions.some((fresh) => fresh.id === s.id)) ?? [];
        page = { ...next, sessions: [...next.sessions, ...retained], before: retained.length ? page?.before ?? null : next.before };
      }
    } catch (e) { if (id === serial) error = explain(e); }
    finally { if (id === serial) loading = false; }
  }

  $effect(() => {
    void historyRevision;
    if (server.link) untrack(() => { void load(); });
  });

  async function relieve(id: number | null): Promise<void> {
    resetting = true;
    try { await api.machine('relieve', { alarm: id }); }
    catch (e) { ui.say(explain(e), true); }
    finally { resetting = false; }
  }
</script>

<Modal title="Alarms" {onclose}>
  {#if faulted}
    <div class="alarm-item"><div class="bar"></div><div><strong>Controller link lost</strong><span class="muted">{faulted}. Reconnect from the top bar.</span></div></div>
  {/if}
  {#each groupAlarms(alarms) as group (group.alarms.map((alarm) => keyOf(alarm.source, alarm.id)).join())}
    {@const titles = alarmTitles(group.alarms)}
    {@const first = group.alarms[0]!}
    <div class="alarm-item">
      <div class="bar" class:warn={!group.alarms.some((alarm) => alarm.blocking)}></div>
      <div class="alarm-text">
        <strong>{titles[0]}</strong>
        {#if titles.length > 1}<span class="also">Also: {titles.slice(1).join(' · ')}</span>{/if}
        <span class="fix">{group.relief.moves_axes ? 'Home the head. Holding the button moves the head up to find its reference.' : first.fix}</span>
        <span class="muted">{group.alarms.some((alarm) => alarm.active) ? 'Active now' : 'Condition cleared · reset to remove'}</span>
      </div>
      {#if group.relief.moves_axes}<HoldButton class="btn btn-move" disabled={resetting || !server.link} onhold={() => relieve(first.id)} title="Moves the Z head to establish its reference">{group.relief.label}</HoldButton>
      {:else}<button class="btn btn-ghost" disabled={resetting || !server.link} onclick={() => relieve(first.id)} title="Resets this alarm">{group.relief.label}</button>{/if}
      <details class="alarm-technical"><summary>Details</summary>{#each group.alarms as alarm (keyOf(alarm.source, alarm.id))}<span>{alarm.label} · {alarm.source}{#if alarm.id !== null} · {alarm.id}{/if}</span>{/each}</details>
    </div>
  {:else}
    {#if !faulted}<div class="all-clear"><i class="ic ic-check"></i><strong>No current alarms</strong></div>{/if}
  {/each}
  {#if alarms.length > 0}
    <div class="live-footer"><span class="muted">Reset does not resume cutting.</span><button class="btn btn-ghost" disabled={resetting || !server.link} onclick={() => relieve(null)}>Reset all</button></div>
  {/if}

  {#if recordingError}<p class="warn-text" role="alert">History could not be saved: {recordingError}</p>{/if}
  <details class="history-panel">
    <summary class="history-heading"><span>History</span><span class="chevron" aria-hidden="true">⌄</span></summary>
    <div class="history-content">
  {#if error}<p class="warn-text" role="alert">{error}</p><button class="btn btn-ghost" onclick={() => load()}>Retry history</button>{/if}
  {#each page?.sessions.filter((session) => session.id === page?.current || session.alarms.length > 0) ?? [] as session (session.id)}
    <section class="history-session">
      <div class="restart"><span>{session.id === page?.current ? 'This session' : 'App restart'} · {date(session.started)}</span></div>
      {#each pastRows(session) as row (row.key)}
        <details class="past-alarm">
          <summary><span>{row.source === 'connection' ? 'Controller link lost' : row.label}</span><span class="when">{time(row.last)}{#if row.count > 1}<b>×{row.count}</b>{/if}</span></summary>
          <div class="alarm-detail">
            {#if row.source === 'connection'}<span>{row.label}</span>{/if}
            <span>First trip {date(row.first)} · last trip {date(row.last)}</span>
            <span>{row.recovered ? `Recovered ${date(row.recovered)}` : 'Present at the last observation in that session'}</span>
            {#if row.reset}<span>Last reset {date(row.reset.at)} · {row.reset.ok ? 'completed' : row.reset.error ?? 'failed'}</span>{/if}
            <span>{row.source}{#if row.id !== null} · {row.id}{/if}</span>
          </div>
        </details>
      {:else}<p class="empty-history muted">No past alarms in this session.</p>{/each}
    </section>
  {/each}
  {#if loading && !page}<p class="muted">Loading history…</p>{/if}
  {#if page?.before}<button class="btn btn-ghost" disabled={loading} onclick={() => load(true)}>{loading ? 'Loading…' : 'Older restarts'}</button>{/if}
    </div>
  </details>
</Modal>

<style>
  .alarm-text { display: grid; gap: 3px; min-width: 0; }
  .alarm-text .also { color: var(--ink-2); font-size: var(--t-sm); }
  .alarm-text .fix { color: var(--ink); font-size: var(--t-sm); }
  .alarm-technical { grid-column: 2 / -1; font-size: var(--t-sm); color: var(--ink-3); }
  .alarm-technical summary { min-height: 44px; padding: 0; }
  .alarm-technical span { display: block; }
  .all-clear { display: flex; gap: 10px; align-items: center; padding: 12px 0; color: var(--move); }
  .live-footer, .history-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .live-footer { font-size: var(--t-sm); }
  .history-panel { margin-top: 4px; border-top: 1px solid var(--line); }
  .history-heading { min-height: 44px; padding: 16px 0 4px; font-size: var(--t-base); font-weight: 600; list-style: none; }
  .history-heading::-webkit-details-marker { display: none; }
  .chevron { color: var(--ink-3); transition: transform .15s; }
  .history-panel[open] > .history-heading .chevron { transform: rotate(180deg); }
  .history-content { display: grid; gap: 12px; padding-top: 10px; }
  .history-session { min-width: 0; }
  .restart { display: flex; align-items: center; gap: 12px; margin: 10px 0 5px; color: var(--ink-3); font-size: var(--t-sm); font-weight: 600; }
  .restart::after { content: ''; flex: 1; height: 1px; background: var(--line); }
  .past-alarm { border-bottom: 1px solid var(--line); }
  summary { min-height: 44px; display: flex; align-items: baseline; gap: 14px; justify-content: space-between; padding: 12px 0; cursor: pointer; font-size: var(--t-sm); }
  summary > span:first-child { min-width: 0; overflow-wrap: anywhere; }
  .when { white-space: nowrap; color: var(--ink-3); font-variant-numeric: tabular-nums; }
  .when b { margin-left: 8px; font-size: var(--t-sm); color: var(--ink-2); }
  .alarm-detail { display: grid; gap: 4px; color: var(--ink-2); font-size: var(--t-sm); padding-bottom: 12px; }
  .empty-history { font-size: var(--t-sm); padding: 8px 0; margin: 0; }
  p { margin: 0; font-size: var(--t-sm); }
</style>
