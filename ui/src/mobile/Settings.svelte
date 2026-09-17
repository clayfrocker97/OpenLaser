<script lang="ts">
  import { ui } from '../stores/ui.svelte';
  import { SETTINGS_PAGES } from '../lib/navigation';
  import { APP_VERSION } from '../lib/version';
  let { open, materials }: { open:(page:number) => void; materials:() => void } = $props();
</script>

<div class="phone-page-title"><h1>Settings</h1><span class="version">v{APP_VERSION}</span></div>
<div class="phone-action-list phone-settings-list">{#each SETTINGS_PAGES as page}<button onclick={() => open(page.id)}>{page.label}<i class="ic ic-arrow-right"></i></button>{/each}</div>
<div class="phone-action-list phone-settings-list"><button onclick={materials}>Material library<i class="ic ic-arrow-right"></i></button><button onclick={() => ui.modal = 'pending'}>Pending changes<i class="ic ic-arrow-right"></i></button></div>

<style>.version { color: var(--ink-3); font-size: 12px; }</style>
