<script lang="ts">
  // The Cutting page: the six controls along the path, in the vendor's
  // pairs, then a line on what else the recipe does. What the machine
  // cannot set is not offered: a nozzle gap without height control, a
  // pressure without a proportional output, a peak output the laser
  // control does not take.
  import Field from './Field.svelte';
  import { cutStart, modeName, type Editor } from '../../lib/recipe';

  let { ed, onpiercing }: { ed: Editor; onpiercing: () => void } = $props();
  const duty = $derived(ed.laser === 'fiber' ? 'CutPower' : 'CutDuty');
  const start = $derived(cutStart(ed.values));
</script>

<div class="section-head"><div><h2>Cutting</h2><p class="muted">The settings used along the cutting path.</p></div></div>
<div class="fields big">
  <Field {ed} key="CutSpeed" big />
  {#if ed.a.height}
    <Field {ed} key="CutHeight" big />
  {:else}
    <div class="fld big"><span class="lbl">Nozzle gap</span><div class="box static"><b>Set by hand</b><span class="unit">no height control</span></div></div>
  {/if}
  <Field {ed} key={duty} big />
  {#if ed.a.gas}<Field {ed} key="CutAirPressure" big />{/if}
  <Field {ed} key="CutFreq" big />
  {#if ed.a.peak}<Field {ed} key="CutPeakCurrent" big />{/if}
</div>
<div class="section-head"><div><h2>Head setup</h2><p class="muted">Fit the nozzle and set optical focus at the head.</p></div></div>
<div class="fields big head-setup">
  <Field {ed} key="OpenLaserNozzleDiameter" big />
  <div class="fld big" class:changed={'OpenLaserNozzleType' in ed.edits}>
    <span class="lbl">Nozzle type</span>
    <div class="chips">
      <button class="chip" class:on={ed.values['OpenLaserNozzleType'] === 'single'} onclick={() => ed.set('OpenLaserNozzleType', 'single')}>Single</button>
      <button class="chip" class:on={ed.values['OpenLaserNozzleType'] === 'double'} onclick={() => ed.set('OpenLaserNozzleType', 'double')}>Double</button>
    </div>
  </div>
  <Field {ed} key="OpenLaserManualFocus" big />
  {#if ed.a.gas || ed.laser === 'co2'}
    <Field {ed} key="CutGasType" big />
  {:else}
    <div class="fld big gas-unavailable"><span class="lbl">Cutting gas</span><p>No gas valve is wired on this machine.</p></div>
  {/if}
</div>
<div class="summary-strip">
  <div><strong>{modeName(ed.values)} · Cut start {start.enabled ? 'on' : 'off'}</strong><p class="muted">Review piercing, then adjust the optional process behaviour.</p></div>
  <button class="btn btn-ghost" onclick={onpiercing}>Edit piercing<i class="ic ic-chev-right"></i></button>
</div>
