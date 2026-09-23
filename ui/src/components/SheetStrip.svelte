<script lang="ts">
  import { api } from '../api/client';
  import { server } from '../stores/server.svelte';
  import { ui } from '../stores/ui.svelte';
  import { explain } from '../lib/format';
  const sheets = $derived(server.doc?.draft?.sheets);
  let busy = $state(false);
  async function choose(index: number): Promise<void> {
    if (busy || !sheets || index === sheets.active) return;
    busy = true;
    try {
      const page = sheets.pages[index]!;
      if (page.job) await api.openJob(page.job); else await api.selectSheet(index);
      ui.selectionEpoch++; ui.setupPanel = null; ui.tab = 'setup';
    } catch (error) { ui.say(explain(error), true); } finally { busy = false; }
  }
</script>

{#if sheets && sheets.pages.length > 1}
  <div class="sheet-strip"><span class="sheet-label">Sheets <small>{sheets.active + 1} / {sheets.pages.length}</small></span><div class="sheet-pages" role="group" aria-label="Numbered sheets">{#each sheets.pages as page, i}<button class:on={sheets.active === i} aria-pressed={sheets.active === i} disabled={busy || ui.nestShown} onclick={() => choose(i)}>Sheet {page.number}<small>{page.parts === null ? 'Saved job' : `${page.parts} parts`}</small></button>{/each}</div></div>
{/if}

<style>
  .sheet-strip { display:flex; align-items:center; gap:18px; padding:10px 18px; border-bottom:1px solid var(--line); min-width:0; }
  .sheet-label { font-size:var(--t-sm); font-weight:700; color:var(--ink-2); flex:none; } .sheet-label small { display:block; font-weight:400; color:var(--ink-3); margin-top:5px; }
  .sheet-pages { display:flex; gap:8px; overflow-x:auto; padding:3px; min-width:0; }
  button { cursor:pointer; flex:none; min-width:100px; min-height:54px; padding:9px 16px; border:1px solid var(--line); border-radius:8px; background:var(--panel-2); color:var(--ink-2); font-size:var(--t-sm); }
  button small { display:block; font-size:var(--t-sm); color:var(--ink-3); margin-top:5px; } button.on { border-color:var(--accent); color:var(--accent); background:var(--accent-soft); }
</style>
