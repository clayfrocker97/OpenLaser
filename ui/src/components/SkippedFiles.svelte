<script lang="ts">
  // Library files the server could not load stay on disk untouched; say so
  // until they are fixed, rather than letting parts or jobs quietly vanish.
  import { server } from '../stores/server.svelte';

  const skipped = $derived(server.doc?.library.skipped ?? []);
</script>

{#if skipped.length}
  <div class="skipped-files" role="alert">
    <details>
      <summary>{skipped.length} library {skipped.length === 1 ? 'file was' : 'files were'} not loaded</summary>
      <p>Each is left untouched in the data folder. Fix or remove it, then restart OpenLaser.</p>
      <ul>{#each skipped as file (file.file)}<li><code>{file.file}</code> · {file.reason}</li>{/each}</ul>
    </details>
  </div>
{/if}
