<script lang="ts">
  // The Piercing page: the mode, then the stages in the order they run.
  // Stage 1 is the highest native bank, as the vendor pierces from the
  // top down. Duration is one value under its three stored names. A
  // stage the file holds no values for asks for each of them; nothing is
  // filled in for the operator.
  import Field from './Field.svelte';
  import { bankHeld, bankOf, bare, shown, duration, durationEdits, isOn, mode, modeName, stageCount, stageKey, type Editor } from '../../lib/recipe';

  let { ed, visible, onvisible, onmode, onrefine }: { ed: Editor; visible: number; onvisible: (stage: number) => void; onmode: () => void; onrefine: () => void } = $props();
  const m = $derived(mode(ed.values));
  const count = $derived(stageCount(ed.values));
  const stage = $derived(Math.min(visible, Math.max(0, count - 1)));
  const bank = $derived(bankOf(count, stage));
  const k = (base: string) => stageKey(base, bank);
  const dur = $derived(duration(ed.values, bank));
  const held = $derived(bankHeld(ed.values, bank));
  const refinements = $derived.by(() => {
    const parts = [isOn(ed.values[k('EnableGradualDrill')]) ? 'Gradual Drill on' : 'Fixed Drill Height', isOn(ed.values[k('BoltDrill_Enable')]) ? 'Bolt Drill on' : 'No Bolt Drill'];
    for (const [base, label] of [['BeforeLaserOffDelay', 'Dwell'], ['AfterLaserOffDelay', 'Afterflow']] as const) {
      const v = ed.values[k(base)];
      if (v && Number(v) > 0) parts.push(`${label} ${bare(k(base), v)} ms`);
    }
    return parts.join(' · ');
  });
  const summary = (i: number) => {
    const b = bankOf(count, i);
    if (!bankHeld(ed.values, b)) return 'needs setup';
    return `${shown(`DrillHeight${b}`, ed.values[`DrillHeight${b}`] ?? '')} · ${bare(`DrillDelay${b}`, duration(ed.values, b).value)} ms`;
  };
</script>

<div class="section-head">
  <div><h2>Piercing</h2><p class="muted">{modeName(ed.values)}{#if m === 'staged'}{' · shown in the order they run'}{/if}</p></div>
  <button class="btn btn-ghost" onclick={onmode}>Change mode</button>
</div>
{#if m === 'none'}
  <div class="empty-mode"><h3>Cut directly</h3><p>This recipe starts cutting without a separate piercing sequence.</p><button class="btn btn-ghost" onclick={onmode}>Choose piercing mode</button></div>
{:else if m === 'smooth'}
  <div class="fields big">
    <Field {ed} key="SmoothPierceDrillHeight" big />
    <Field {ed} key="SmoothPierceDrillPower" big />
    <Field {ed} key="SmoothPierceDrillFreq" big />
    {#if ed.a.peak}<Field {ed} key="SmoothPierceDrillPeakCurrent" big />{/if}
  </div>
  <div class="summary-strip"><div><strong>Continuous transition into cutting</strong><p class="muted">Uses cutting gas; smooth duration is inactive on this machine.</p></div></div>
{:else}
  <div class="steps">
    {#each Array.from({ length: count }, (_, i) => i) as i (i)}
      <button class="step" class:on={stage === i} class:idle={!bankHeld(ed.values, bankOf(count, i))} onclick={() => onvisible(i)}><span class="num">{i + 1}</span><span><b>Stage {i + 1}</b><small>{summary(i)}</small></span></button>
    {/each}
  </div>
  {#if !held}<div class="notice warn">Stage {stage + 1} has no stored values. Enter each one; nothing is filled in for you.</div>{/if}
  {#if dur.conflict}<div class="notice warn">The stored duration names disagree ({dur.present.join(', ')}); setting the duration writes one value to all of them.</div>{/if}
  <div class="fields big">
    <Field {ed} key={k('DrillHeight')} big />
    <Field {ed} key={k('DrillDelay')} big value={dur.value} oncommit={(t) => ed.setMany(durationEdits(ed.values, bank, t))} />
    <Field {ed} key={k('DrillPower')} big />
    {#if ed.a.gas}<Field {ed} key={k('DrillGasPressure')} big />{/if}
    <Field {ed} key={k('DrillFreq')} big />
    {#if ed.a.peak}<Field {ed} key={k('DrillPeakCurrent')} big />{/if}
    {#if ed.a.gas}<div class="wide"><Field {ed} key={k('DrillGasType')} /></div>{/if}
  </div>
  <button class="launcher" onclick={onrefine}><div><strong>Stage {stage + 1} refinements</strong><small>{refinements}</small></div><i class="ic ic-chev-right"></i></button>
{/if}
