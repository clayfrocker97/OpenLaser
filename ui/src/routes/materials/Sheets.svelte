<script lang="ts">
  import { quantity } from '../../lib/units.svelte';
  // The sheets the recipe pages open: the piercing mode, a stage's
  // refinements, each process option, the review of staged changes and
  // the imported values. A sheet edits the same staged values as the
  // pages; closing it keeps them.
  import Modal from '../../components/Modal.svelte';
  import Field from './Field.svelte';
  import Inspector from './Inspector.svelte';
  import { GAS, bankOf, shown, curvePoints, cutStart, cutStartEdits, field, headMode, isOn, manuType, mode, modeEdits, stageCount, stageKey, stageOf, type Editor, type HeadMode, type Mode } from '../../lib/recipe';
  import type { RecipeView } from '../../api';

  let { ed, which, recipe, visible, recipes, film, onfilm, onclose }: {
    ed: Editor;
    which: string;
    recipe: RecipeView;
    /** The visible piercing stage, for its refinements. */
    visible: number;
    recipes: RecipeView[];
    /** The film process as staged. */
    film: RecipeView | null;
    onfilm: (id: string | null) => void;
    onclose: () => void;
  } = $props();

  const v = $derived(ed.values);
  const m = $derived(mode(v));
  const count = $derived(stageCount(v));
  const bank = $derived(bankOf(count, Math.min(visible, Math.max(0, count - 1))));
  const k = (base: string) => stageKey(base, bank);
  const caps = $derived(ed.a.caps);
  const head = $derived(headMode(v));
  const start = $derived(cutStart(v));
  const TITLES: Record<string, string> = {
    mode: 'Piercing mode', review: 'Review changes', inspector: 'Imported values', start: 'Cut start', end: 'Cut end', timing: 'Timing',
    head: 'Head movement', curves: 'Speed compensation', residue: 'Slag removal', pre: 'Batch pre-piercing', film: 'Film removal', shift: 'Contour shift',
  };
  const title = $derived(which === 'refine' ? `Stage ${Math.min(visible, Math.max(0, count - 1)) + 1} refinements` : TITLES[which] ?? which);
  const stages = $derived(Math.max(1, count));
  const setMode = (to: Mode, n = stages) => ed.setMany(modeEdits(v, to, n));
  const setHead = (to: HeadMode) => ed.set('ManuType', manuType(to, to === 'follow' ? count : 0));
  const curvePath = (nodes: string) => curvePoints(nodes).map(([x, p], i) => `${i ? 'L' : 'M'}${x} ${100 - p}`).join(' ');
  /** The film processes on offer: the other recipes for this laser. */
  const candidates = $derived(recipes.filter((r) => r.laser === recipe.laser && r.id !== recipe.id).sort((a, b) => a.name.localeCompare(b.name) || a.thickness_mm - b.thickness_mm));
  /** Where a change belongs, for the review. */
  const context = (key: string) => { const s = stageOf(key); return s === null ? '' : `Stage ${count - s} · `; };
  const changes = $derived(Object.keys(ed.edits).sort().map((key) => ({ key, label: `${context(key)}${field(key).label}`, from: shown(key, recipe.attributes[key] ?? ''), to: shown(key, ed.edits[key] ?? '') })));
  const filmChanged = $derived((film?.id ?? null) !== (recipe.film ?? null));
  /** The two speed curves: the switch, the nodes and the smoothing of each. */
  const CURVES: Array<[string, string, string]> = [['PowerAdjustWithSpeed', 'PWMCurveNodes', 'PowerCurveSmoothType'], ['FreqAdjustWithSpeed', 'FreqCurveNodes', 'FreqCurveSmoothType']];
</script>

<Modal {title} wide={which === 'inspector' || which === 'curves'} {onclose}>
  <div class="sheet-body">
    {#if which === 'mode'}
      {#each [['none', 'No piercing', 'Cut directly.'], ['staged', 'Staged piercing', 'One to five stages, highest first.'], ['smooth', 'Smooth piercing', 'Transition into cutting with the cutting gas.']] as [id, name, help] (id)}
        <button class="mode-choice" class:on={m === id} disabled={id === 'smooth' && isOn(v['PreDrill'])} onclick={() => setMode(id as Mode)}><strong>{name}</strong><span>{help}</span></button>
      {/each}
      {#if isOn(v['PreDrill'])}<div class="notice info">Turn off batch pre-piercing to use smooth piercing.</div>{/if}
      {#if m === 'staged'}
        <div class="field"><span class="lbl">Number of stages</span><div class="seg block">{#each [1, 2, 3, 4, 5] as n (n)}<button class:on={count === n} onclick={() => setMode('staged', n)}>{n}</button>{/each}</div></div>
        <p class="muted">Unused stages keep their values. New stages need all fields.</p>
      {:else if head !== 'follow'}
        <p class="muted">Staged piercing enables sheet following.</p>
      {/if}
    {:else if which === 'refine'}
      <Field {ed} key={k('EnableGradualDrill')} row note="Move toward the next stage or cutting height." />
      <Field {ed} key={k('BoltDrill_Enable')} row note="Change from the stage's duty and frequency to the endpoints below." />
      <div class="fields"><Field {ed} key={k('BoltDrill_Power')} /><Field {ed} key={k('BoltDrill_Freq')} /><Field {ed} key={k('BeforeLaserOffDelay')} /><Field {ed} key={k('AfterLaserOffDelay')} /></div>
      <p class="muted">Duration controls the transition. Optical focus is manual.</p>
    {:else if which === 'review'}
      <div class="review">
        {#each changes as c (c.key)}<div class="change"><span>{c.label} <span class="mono muted">{c.key}</span></span><b>{c.from} → {c.to}</b></div>{/each}
        {#if filmChanged}<div class="change"><span>Film process</span><b>{recipes.find((r) => r.id === recipe.film)?.name ?? 'none'} → {film?.name ?? 'none'}</b></div>{/if}
        {#if !changes.length && !filmChanged}<p class="muted">There are no staged changes.</p>{/if}
      </div>
    {:else if which === 'inspector'}
      <Inspector {ed} {recipe} />
    {:else if which === 'start'}
      <div class="toggle-row"><div><strong>Cut start</strong><small>A slower region at the start of every contour.</small></div><button
        class="switch" class:on={start.enabled} onclick={() => ed.setMany(cutStartEdits(v, { enabled: !start.enabled }))} aria-label="Cut start"></button></div>
      {#if start.legacy}<div class="notice info">Editing replaces the legacy slow-start settings.</div>{/if}
      <div class="fields">
        <Field {ed} key="UD_UpLen" value={start.length} oncommit={(t) => ed.setMany(cutStartEdits(v, { length: t }))} />
        <Field {ed} key="UD_UpSpeed" value={start.speed} oncommit={(t) => ed.setMany(cutStartEdits(v, { speed: t }))} />
      </div>
      <Field {ed} key="UD_UpAdvEnable" row />
      <div class="fields"><Field {ed} key="UD_UpDuty" /><Field {ed} key="UD_UpFreq" /></div>
    {:else if which === 'end'}
      <Field {ed} key="UD_DownEnable" row note="A slower region at the end of every contour." />
      <div class="fields"><Field {ed} key="UD_DownLen" /><Field {ed} key="UD_DownSpeed" /></div>
      <Field {ed} key="UD_DownAdvEnable" row />
      <div class="fields"><Field {ed} key="UD_DownDuty" /><Field {ed} key="UD_DownFreq" /></div>
    {:else if which === 'timing'}
      <p class="muted">Delays before cutting, before laser off, and after laser off.</p>
      <div class="fields"><Field {ed} key="LaserOnDelay" /><Field {ed} key="LaserOffBeforeDelay" /><Field {ed} key="LaserOffAfterDelay" /></div>
    {:else if which === 'head'}
      {#if ed.a.height}
        <div class="field"><span class="lbl">Head mode</span><div class="seg block"><button
            class:on={head === 'follow'} onclick={() => setHead('follow')}>Follow the sheet</button><button
            class:on={head === 'fixed'} onclick={() => setHead('fixed')}>Fixed height</button><button
            class:on={head === 'absolute'} onclick={() => setHead('absolute')}>Absolute height</button></div></div>
        {#if head !== 'follow'}<p class="muted">A fixed or absolute head does not pierce in stages.</p>{/if}
        <div class="fields"><Field {ed} key="UpHeight" />{#if head === 'absolute'}<Field {ed} key="AdvFixHeightCutPos" />{/if}</div>
        <Field {ed} key="NoFollow" row value={isOn(v['NoFollow']) ? '0' : '1'} oncommit={(t) => ed.set('NoFollow', t === '1' ? '0' : '1')} />
        <Field {ed} key="ShortDistNoUp" row note={caps?.short_transfer_mm != null ? `Under the machine's short-transfer distance of ${quantity(caps.short_transfer_mm, 'mm')}.` : "Under the machine's short-transfer distance."} />
      {:else}
        <div class="notice info">This laser runs without height control: the head is set by hand.</div>
      {/if}
      {#if ed.a.gas}
        <Field {ed} key="ShortDistGasKeepOn" row note="Independent of whether the head lifts." />
        <Field {ed} key="NoCloseGasInManu" row note="Gas closes at job end; selection changes still switch valves." />
      {/if}
      {#if ed.a.height}
        <h3>Following response · expert</h3>
        <div class="fields"><Field {ed} key="ZFVibAbatType" /><Field {ed} key="ZFVibAbat_Level" /><Field {ed} key="ZFVibAbat_Level_Thick" /></div>
      {/if}
    {:else if which === 'curves'}
      <div class="curves">
        {#each CURVES as [flagKey, nodesKey, smoothKey] (flagKey)}
          <div class="curve">
            <Field {ed} key={flagKey} row />
            {#if v[nodesKey]}
              <svg class="chart" viewBox="-14 -6 124 118" aria-hidden="true">
                <rect x="0" y="0" width="100" height="100"/>
                {#each [20, 40, 60, 80] as g (g)}<line x1={g} y1="0" x2={g} y2="100"/><line x1="0" y1={g} x2="100" y2={g}/>{/each}
                <path d={curvePath(v[nodesKey] ?? '')}/>
                {#each curvePoints(v[nodesKey] ?? '') as [x, p] (x)}<circle cx={x} cy={100 - p} r="2.2"/>{/each}
                <text x="-4" y="104" text-anchor="end">0</text><text x="100" y="110" text-anchor="middle">V %</text><text x="-4" y="4" text-anchor="end">100</text>
              </svg>
            {/if}
            <div class="fields"><Field {ed} key={nodesKey} note="Speed and output pairs in percent, from 0 to 100." /><Field {ed} key={smoothKey} note="Display smoothing only; cutting uses the curve points." /></div>
          </div>
        {/each}
      </div>
    {:else if which === 'residue'}
      <Field {ed} key="CleanResidue_Enable" row note="A spiral over the pierce before the cut." />
      {#if m !== 'staged'}<div class="notice warn">Slag removal runs only after staged piercing; the settings are kept.</div>{/if}
      <div class="fields">
        <Field {ed} key="CleanResidue_WorkH" /><Field {ed} key="CleanResidue_WorkV" /><Field {ed} key="CleanResidue_Power" />
        {#if ed.a.gas}<Field {ed} key="CleanResidue_GasP" />{/if}
        <Field {ed} key="CleanResidue_Freq" />{#if ed.a.peak}<Field {ed} key="CleanResidue_PeakCurrent" />{/if}
        <Field {ed} key="CleanResidue_WorkR" /><Field {ed} key="CleanResidue_SpiralTimes" />
        {#if ed.a.gas}<div class="wide"><Field {ed} key="CleanResidue_GasType" /></div>{/if}
      </div>
    {:else if which === 'pre'}
      {#if m === 'smooth'}<div class="notice warn">Batch pre-piercing is unavailable with smooth piercing.</div>{/if}
      {#if m === 'none'}<div class="notice info">Needs staged piercing; the switch is kept as data until then.</div>{/if}
      <Field {ed} key="PreDrill" row note="Pierce a batch of contours before cutting them." />
      <p><strong>{caps?.pre_pierce_batch ?? 'Not set'}</strong>{caps?.pre_pierce_batch != null ? ' contours per batch' : ''} <span class="muted">· from the machine's operating settings (PreDrillMaxNum)</span></p>
      <Field {ed} key="AfterPreDrillMustDrillBeforeCut" row />
      <Field {ed} key="PreDrillIsNotUp" row note="Lifts at the batch boundary and after the final point." />
    {:else if which === 'film'}
      <Field {ed} key="WithFilm" row note="A separate pass over each contour's bare geometry before its cut." />
      {#if isOn(v['WithFilm']) && !film}<div class="notice warn">Select a film process; compiling refuses without one.</div>{/if}
      <div class="field"><span class="lbl">Film process</span>
        <div class="pick-list">
          <button class="from" class:on={!film} onclick={() => onfilm(null)}><span>None</span></button>
          {#each candidates as r (r.id)}<button class="from" class:on={film?.id === r.id} onclick={() => onfilm(r.id)}><span>{r.name}</span><small>{quantity(r.thickness_mm, 'mm')} · {r.gas}</small></button>{/each}
        </div>
      </div>
      <p class="muted">Film removal uses its selected recipe.</p>
      <p><strong>{caps?.film_batch ? `${caps.film_batch} contours per batch` : 'Batch: whole job'}</strong> <span class="muted">· from the machine's operating settings (ClearUpFilmNum_Pre)</span></p>
    {:else if which === 'shift'}
      <Field {ed} key="EnableContourShift" row note="Shifts cutting and film paths; keeps the sheet origin." />
      <div class="fields"><Field {ed} key="ContourShiftXDist" /><Field {ed} key="ContourShiftYDist" /></div>
    {/if}
    {#if which !== 'review' && which !== 'inspector'}
      <p class="muted">Gas selections: {GAS.filter((_, i) => ed.a.gases.includes(i)).join(', ') || 'none wired'}. Save to keep edits.</p>
    {/if}
  </div>
</Modal>
