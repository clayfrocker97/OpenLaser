<script lang="ts">
  import Modal from './Modal.svelte';
  import { access } from '../lib/access.svelte';
</script>

<Modal title="Local network" onclose={() => access.manage = false}>
  <div class="network-address">
    <span>OpenLaser on your network</span>
    <strong>{access.info?.address.replace('http://', '')}</strong>
    <p>{access.info?.discovery.error ?? (access.info?.discovery.ready ? 'Open this address on any phone, tablet or computer connected to the same network.' : access.info?.discovery.enabled ? 'Announcing the local address…' : 'Local network sharing is off.')}</p>
  </div>
</Modal>

<style>
  .network-address { display:grid; gap:14px; }.network-address span { font-size:13px; color:var(--ink-2); }.network-address strong { font-size:27px; letter-spacing:-.02em; overflow-wrap:anywhere; }.network-address p { color:var(--ink-3); font-size:14px; line-height:1.6; }
</style>
