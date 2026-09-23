<script lang="ts">
  import Modal from './Modal.svelte';
  import { osk } from '../lib/osk.svelte';

  function commit(event: SubmitEvent): void {
    event.preventDefault();
    osk.key('done');
  }
</script>

{#if osk.open}
  <Modal title={osk.label} onclose={() => osk.close()}>
    <form onsubmit={commit}>
      <label><span>{osk.label}{osk.unit ? ` · ${osk.unit}` : ''}</span><input aria-label={osk.label} inputmode={osk.kind === 'num' ? 'decimal' : 'text'} autocomplete="off" bind:value={osk.value} /></label>
      <button class="btn btn-primary lg block" type="submit">Done</button>
    </form>
  </Modal>
{/if}

<style>
  form { display:grid; gap:20px; }
  label { display:grid; gap:10px; color:var(--ink-2); font-size:var(--t-sm); }
  input { height:58px; width:100%; min-width:0; padding:0 14px; border:1px solid var(--line-2); border-radius:12px; background:var(--panel-2); color:var(--ink); font-size:var(--t-lg); user-select:text; }
</style>
