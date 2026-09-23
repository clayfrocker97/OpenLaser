<script lang="ts">
  import { displayNumber, unitLabel } from '../../lib/units.svelte';
  // One machining feature's settings: every field the server takes, in
  // the words the operator uses. Places on the drawing are picked on the
  // canvas; the panel only starts and ends the picking.
  import { featureEdits } from '../../stores/feature-edits';
  import { server } from '../../stores/server.svelte';
  import { ui, type FeatureId } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { ago, explain, plural, recipeLabel } from '../../lib/format';
  import { TOOLS, WORDS, defaultBridges, defaultCommon, defaultCooling, defaultJoints, defaultKerf, defaultLeads, isOn } from '../../lib/features';
  import CutOrderPreview from './CutOrderPreview.svelte';
  import type { Features, Lead, Spot } from '../../api';

  let { id, ontoggle, selectedContours, orderProgress = $bindable(0), compact = false }: { id: FeatureId; ontoggle: () => void; selectedContours: number[]; orderProgress?: number; compact?: boolean } = $props();
  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft!);
  const tool = $derived(TOOLS.find((t) => t.id === id)!);
  const features = $derived(featureEdits.value(draft));
  const on = $derived(isOn(features, id));
  let tab = $state(0);
  $effect(() => { void id; tab = 0; });

  function set(change: (f: Features) => void): void {
    featureEdits.change(change).catch((error) => ui.say(explain(error), true));
  }

  function num(label: string, value: number, unit: string, apply: (v: number) => void): void {
    osk.number(label, value, unit, (v) => apply(Math.max(0, v)));
  }

  /** Starts placing on the drawing; the canvas takes it from here. */
  function pick(feature: 'joints' | 'cooling' | 'start' | 'bridges' | 'order'): void {
    ui.picking = { feature, first: null, order: [], revision: draft.revision, features: structuredClone($state.snapshot(features)), marks: [] };
  }

  function percentages(feature: 'joints' | 'cooling'): void {
    const placement = features[feature]?.placement;
    const values = placement && 'manual' in placement ? [...new Set(placement.manual.map(s => +(s.fraction * 100).toFixed(3)))].join(', ') : '';
    osk.text('Positions on each contour (%)', values, (text) => {
      const values = text.trim() ? text.split(',').map(v => v.trim() ? Number(v.trim()) : NaN) : [];
      if (values.some(v => !Number.isFinite(v) || v < 0 || v > 100)) { ui.say('Enter comma-separated percentages from 0 to 100.', true); return; }
      const manual = draft.placed.flatMap((_, contour) => [...new Set(values)].map(v => ({ contour, fraction: v / 100 })));
      set(f => { const target = f[feature]; if (target) target.placement = { manual }; });
    });
  }
  function typedOrder(): void {
    const current = features.order.strategy;
    osk.text('Contour IDs in cutting order (starting at 1)', typeof current === 'object' ? current.manual.map(v => v + 1).join(', ') : '', (text) => {
      const order = text.split(',').map(v => v.trim() ? Number(v.trim()) - 1 : NaN);
      const all = [...new Set(draft.preview?.contours.flatMap(c => c.sources) ?? [])];
      if (order.length !== all.length || new Set(order).size !== order.length || order.some(v => !Number.isInteger(v) || !all.includes(v))) { ui.say(`Enter each active contour ID once: ${all.map(v => v + 1).join(', ')}.`, true); return; }
      set(f => { f.order.strategy = { manual: order }; });
    });
  }

  const defaultLead = (): Lead => ({ shape: 'line', length: 2, radius: 1, angle: 45 });
  const spots = (placement: { manual: Spot[] } | object): Spot[] => ('manual' in placement ? placement.manual : []);

  function reset(): void {
    set((f) => {
      if (id === 'leads') f.leads = defaultLeads();
      if (id === 'joints') f.joints = defaultJoints();
      if (id === 'cooling') f.cooling = defaultCooling();
      if (id === 'kerf') f.kerf = defaultKerf();
      if (id === 'bridges') f.bridges = defaultBridges();
      if (id === 'common' && f.common) f.common = defaultCommon(f.common.contours);
      if (id === 'order') f.order = { strategy: 'as_drawn', inner_first: true, circles_first: false, spread_heat: false };
      if (id === 'start') { f.start = { position: 'automatic', direction: 'keep', spots: [] }; f.seam = 'seal'; }
    });
  }
</script>

{#snippet stepper(label: string, value: number, unit: string, step: number, apply: (v: number) => void)}
  <div class="param"><div class="lbl">{label}</div>
    <div class="stepper"><button onclick={() => apply(Math.max(0, +(value - step).toFixed(3)))} disabled={!on}>−</button><div class="val tappable" data-numpad role="button" aria-disabled={!on} tabindex={on ? 0 : -1} onclick={() => { if (on) num(label, value, unit, apply); }} onkeydown={() => undefined}>{displayNumber(value, unit, step < 0.1 ? 2 : 1)}<small>{unitLabel(unit)}</small></div><button onclick={() => apply(+(value + step).toFixed(3))} disabled={!on}>+</button></div>
  </div>
{/snippet}

{#snippet chips(label: string, options: Array<[string, string]>, current: string, apply: (v: string) => void)}
  <div class="field"><div class="lbl" style="font-weight:600;font-size:var(--t-base)">{label}</div><div class="chips">{#each options as [value, text]}<button class="chip" class:on={current === value} onclick={() => apply(value)} disabled={!on}>{text}</button>{/each}</div></div>
{/snippet}

{#snippet toggle(label: string, value: boolean, apply: (v: boolean) => void)}
  <div class="param"><div class="lbl">{label}</div><button class="switch" class:on={value} onclick={() => apply(!value)} disabled={!on} style="justify-self:end" aria-label={label}></button></div>
{/snippet}

{#snippet placed(count: number, what: string, feature: 'joints' | 'cooling' | 'start' | 'bridges' | 'order', clear: () => void)}
  <div class="param"><div class="lbl">{count} {what}{count === 1 ? '' : 's'} on the drawing</div>
    <div class="row"><button class="btn btn-ghost" onclick={clear} disabled={!on || count === 0}>Clear</button><button class="btn btn-soft" onclick={() => pick(feature)} disabled={!on}>Pick</button></div>
  </div>
{/snippet}

{#snippet tabs(names: string[])}
  <div class="seg block">{#each names as name, i}<button class:on={tab === i} onclick={() => (tab = i)}>{name}</button>{/each}</div>
{/snippet}

{#if !compact || tool.optional}<div class="feat-head"><h2>{compact ? 'Enabled' : tool.name}</h2>{#if tool.optional}<button class="switch" class:on={on} onclick={ontoggle} aria-label="On"></button>{/if}{#if !compact}<button class="btn btn-ghost" onclick={() => (ui.setupPanel = null)}>Close</button>{/if}</div>{/if}
<p class="feat-desc">{tool.desc}</p>
{#if id === 'order'}<CutOrderPreview preview={draft.preview} disabled={!!ui.picking} bind:progress={orderProgress} />{/if}
{#if featureEdits.error}<p class="muted">{featureEdits.error} <button class="link" onclick={() => featureEdits.discard()}>Discard refused edits</button></p>{/if}
{#if id === 'leads'}{@render tabs(['Entry', 'Exit'])}{/if}
{#if id === 'leads'}<p class="muted" style="font-size:var(--t-sm)">Drag a lead to edit it. Selected copies update the matching lead.</p>{/if}
{#if id === 'joints' || id === 'start'}{@render tabs(['Basic', 'More'])}{/if}
<div class="stack" style="opacity:{on ? 1 : 0.45}">
  {#if id === 'leads'}
    {@const leads = features.leads ?? defaultLeads()}
    {@const lead = tab === 0 ? leads.entry : leads.exit}
    {@render toggle(tab === 0 ? 'Lead-in' : 'Lead-out', lead !== null, (v) => set((f) => { const l = f.leads ?? defaultLeads(); if (tab === 0) l.entry = v ? defaultLead() : null; else l.exit = v ? defaultLead() : null; f.leads = l; }))}
    {#if lead}
      {@render chips('Shape', [['line', 'Line'], ['arc', 'Arc'], ['line_arc', 'Line and arc']], lead.shape, (v) => set((f) => { const l = tab === 0 ? f.leads!.entry! : f.leads!.exit!; l.shape = v as Lead['shape']; }))}
      {#if lead.shape !== 'arc'}{@render stepper('Length', lead.length, 'mm', 0.5, (v) => set((f) => { const l = tab === 0 ? f.leads!.entry! : f.leads!.exit!; l.length = v; }))}{/if}
      {#if lead.shape !== 'line'}{@render stepper('Arc radius', lead.radius, 'mm', 0.5, (v) => set((f) => { const l = tab === 0 ? f.leads!.entry! : f.leads!.exit!; l.radius = v; }))}{/if}
      {@render stepper(lead.shape === 'arc' ? 'Arc sweep' : 'Angle', lead.angle, '°', 15, (v) => set((f) => { const l = tab === 0 ? f.leads!.entry! : f.leads!.exit!; l.angle = v; }))}
    {/if}
    {@render chips('Side', [['auto', 'Auto'], ['inside', 'Inside'], ['outside', 'Outside']], leads.side, (v) => set((f) => { f.leads!.side = v as typeof leads.side; }))}
    {@render toggle('Closed contours only', leads.closed_only, (v) => set((f) => { f.leads!.closed_only = v; }))}
    {#if leads.overrides?.length}
      <p class="muted" style="font-size:var(--t-sm)">{leads.overrides.length} contour{leads.overrides.length === 1 ? '' : 's'} with individual lead settings</p>
      <button class="btn btn-soft" disabled={!on || !leads.overrides.some(edited => selectedContours.includes(edited.location.contour))} onclick={() => set(f => { f.leads!.overrides = f.leads!.overrides.filter(edited => !selectedContours.includes(edited.location.contour)); })}>Use job defaults for selection</button>
      <button class="btn btn-ghost" disabled={!on} onclick={() => set(f => { f.leads!.overrides = []; })}>Reset all individual leads</button>
    {/if}
  {:else if id === 'joints'}
    {@const joints = features.joints ?? defaultJoints()}
    {@const placement = joints.placement}
    {@const kind = 'count' in placement ? 'count' : 'spacing' in placement ? 'spacing' : 'across_x' in placement ? 'across_x' : 'across_y' in placement ? 'across_y' : 'manual'}
    {#if tab === 0}
      {@render chips('Placement', [['count', 'Per contour'], ['spacing', 'By spacing'], ['across_x', 'Across X'], ['across_y', 'Across Y'], ['manual', 'Picked']], kind, (v) => set((f) => { f.joints!.placement = v === 'count' ? { count: 2 } : v === 'spacing' ? { spacing: 100 } : v === 'across_x' ? { across_x: 2 } : v === 'across_y' ? { across_y: 2 } : { manual: spots(f.joints!.placement) }; }))}
      {#if 'count' in placement}{@render stepper('Joints per contour', placement.count, '', 1, (v) => set((f) => { f.joints!.placement = { count: Math.max(1, Math.round(v)) }; }))}{/if}
      {#if 'spacing' in placement}{@render stepper('Spacing', placement.spacing, 'mm', 10, (v) => set((f) => { f.joints!.placement = { spacing: v }; }))}{/if}
      {#if 'across_x' in placement}{@render stepper('Grid lines across X', placement.across_x, '', 1, (v) => set((f) => { f.joints!.placement = { across_x: Math.max(1, Math.round(v)) }; }))}{/if}
      {#if 'across_y' in placement}{@render stepper('Grid lines across Y', placement.across_y, '', 1, (v) => set((f) => { f.joints!.placement = { across_y: Math.max(1, Math.round(v)) }; }))}{/if}
      {#if 'manual' in placement}{@render placed(placement.manual.length, 'joint', 'joints', () => set((f) => { f.joints!.placement = { manual: [] }; }))}{/if}
      <button class="btn btn-soft" onclick={() => percentages('joints')} disabled={!on}>Enter positions (%)…</button>
      {@render stepper('Joint width', joints.width, 'mm', 0.1, (v) => set((f) => { f.joints!.width = v; }))}
      {@render stepper('Skip contours under', joints.minimum_size, 'mm', 10, (v) => set((f) => { f.joints!.minimum_size = v; }))}
      {@render toggle('Outer contours only', joints.outer_only, (v) => set((f) => { f.joints!.outer_only = v; }))}
    {:else}
      {@render toggle('Joint at an open contour’s start', joints.open_start, (v) => set((f) => { f.joints!.open_start = v; }))}
      {@render chips('In the joint', [['laser_off', 'Laser off'], ['power', 'Low power']], joints.behaviour === 'laser_off' ? 'laser_off' : 'power', (v) => set((f) => { f.joints!.behaviour = v === 'laser_off' ? 'laser_off' : { power: 10 }; }))}
      {#if typeof joints.behaviour !== 'string'}{@render stepper('Joint power', joints.behaviour.power, '%', 5, (v) => set((f) => { f.joints!.behaviour = { power: Math.min(100, v) }; }))}{/if}
      {@render toggle('Slow down in the joint', joints.slow_speed !== null, (v) => set((f) => { f.joints!.slow_speed = v ? 5 : null; }))}
      {#if joints.slow_speed !== null}{@render stepper('Joint speed', joints.slow_speed, 'mm/s', 1, (v) => set((f) => { f.joints!.slow_speed = Math.max(0.1, v); }))}{/if}
      {@render toggle('Pierce again after each joint', joints.repierce, (v) => set((f) => { f.joints!.repierce = v; }))}
      <p class="muted" style="font-size:var(--t-sm);margin:0">Piercing again needs a recipe with pierce stages.</p>
    {/if}
  {:else if id === 'cooling'}
    {@const cooling = features.cooling ?? defaultCooling()}
    <button class="btn btn-soft" onclick={() => percentages('cooling')} disabled={!on}>Enter positions (%)…</button>
    {@render stepper('Dwell', cooling.dwell / 1000, 's', 0.1, (v) => set((f) => { f.cooling!.dwell = Math.max(1, Math.round(v * 1000)); }))}
    {@render chips('Placement', [['automatic', 'Automatic'], ['manual', 'Picked']], 'automatic' in cooling.placement ? 'automatic' : 'manual', (v) => set((f) => { f.cooling!.placement = v === 'automatic' ? { automatic: { at_start: true, corners_below: 60 } } : { manual: spots(f.cooling!.placement) }; }))}
    {#if 'automatic' in cooling.placement}
      {@const auto = cooling.placement.automatic}
      {@render toggle('Cool after the pierce', auto.at_start, (v) => set((f) => { const p = f.cooling!.placement; if ('automatic' in p) p.automatic.at_start = v; }))}
      {@render toggle('Cool at sharp corners', auto.corners_below !== null, (v) => set((f) => { const p = f.cooling!.placement; if ('automatic' in p) p.automatic.corners_below = v ? 60 : null; }))}
      {#if auto.corners_below !== null}{@render stepper('Corners sharper than', auto.corners_below, '°', 5, (v) => set((f) => { const p = f.cooling!.placement; if ('automatic' in p) p.automatic.corners_below = Math.min(180, v); }))}{/if}
    {:else}
      {@render placed(cooling.placement.manual.length, 'stop', 'cooling', () => set((f) => { f.cooling!.placement = { manual: [] }; }))}
    {/if}
  {:else if id === 'kerf'}
    {@const kerf = features.kerf ?? defaultKerf()}
    {@render stepper('Kerf width', kerf.width, 'mm', 0.01, (v) => set((f) => { f.kerf!.width = v; }))}
    {@render chips('Waste side', [['auto', 'Auto'], ['inside', 'Inside'], ['outside', 'Outside']], kerf.side, (v) => set((f) => { f.kerf!.side = v as typeof kerf.side; }))}
  {:else if id === 'bridges'}
    {@const bridges = features.bridges ?? defaultBridges()}
    {@render stepper('Channel width', bridges.width, 'mm', 0.5, (v) => set((f) => { f.bridges!.width = v; }))}
    {@render placed(bridges.connections.length, 'bridge', 'bridges', () => set((f) => { f.bridges!.connections = []; }))}
    <div class="param"><div class="lbl">Tap two contours to join, or one contour twice to split.</div><button class="btn btn-ghost" onclick={() => set((f) => { f.bridges!.connections.pop(); })} disabled={!on || bridges.connections.length === 0}>Undo last</button></div>
  {:else if id === 'common'}
    {@const common = features.common ?? defaultCommon(selectedContours)}
    <div class="param"><div class="lbl">{features.common ? `${plural(common.contours.length, 'contour')} in this common-edge group` : 'Select adjacent contours on the drawing, then switch on common edges.'}</div></div>
    <button class="btn btn-soft" disabled={!on || selectedContours.length < 2 || !!ui.picking} onclick={() => set(f => { if (f.common) f.common.contours = [...selectedContours]; })}>Use selected contours ({selectedContours.length})</button>
    {@render stepper('Matching tolerance', common.tolerance, 'mm', 0.01, (v) => set(f => { f.common!.tolerance = v; }))}
    {@render toggle('Allow overcut between remaining spans', common.allow_overcut, (v) => set(f => { f.common!.allow_overcut = v; }))}
    {#if draft.error}<p class="warn-text">{draft.error}</p>{/if}
  {:else if id === 'order'}
    {@const order = features.order}
    {@const strategy = typeof order.strategy === 'string' ? order.strategy : 'manual'}
    {@render chips('Strategy', [['as_drawn', WORDS['as_drawn']!], ['nearest', WORDS['nearest']!], ['left_to_right', WORDS['left_to_right']!], ['right_to_left', WORDS['right_to_left']!], ['bottom_to_top', WORDS['bottom_to_top']!], ['top_to_bottom', WORDS['top_to_bottom']!], ['manual', 'Picked']], strategy, (v) => { if (v === 'manual') pick('order'); else set((f) => { f.order.strategy = v as Exclude<typeof order.strategy, object>; }); })}
    {#if typeof order.strategy !== 'string'}
      <div class="param"><div class="lbl">Contours cut in the order they were tapped</div><button class="btn btn-soft" onclick={() => pick('order')}>Pick again</button></div>
    {/if}
    <button class="btn btn-soft" onclick={typedOrder}>Enter contour order…</button>
    {@render toggle('Inner contours first', order.inner_first, (v) => set((f) => { f.order.inner_first = v; }))}
    {@render toggle('Circles first', order.circles_first, (v) => set((f) => { f.order.circles_first = v; }))}
    {@render toggle('Spread heat', order.spread_heat, (v) => set((f) => { f.order.spread_heat = v; }))}
  {:else if id === 'start'}
    {@const start = features.start}
    {#if tab === 0}
      {@render chips('Start point', [['keep', WORDS['keep']!], ['automatic', WORDS['automatic']!], ['manual', 'Along each contour']], typeof start.position === 'string' ? start.position : 'manual', (v) => set((f) => { f.start.position = v === 'manual' ? { manual: 0.5 } : (v as 'keep' | 'automatic'); }))}
      {#if typeof start.position !== 'string'}{@render stepper('Along the contour', Math.round(start.position.manual * 100), '%', 5, (v) => set((f) => { f.start.position = { manual: Math.min(100, v) / 100 }; }))}{/if}
      {@render placed(start.spots.length, 'chosen start', 'start', () => set((f) => { f.start.spots = []; }))}
      {@render chips('Direction', [['keep', WORDS['keep']!], ['clockwise', 'Clockwise'], ['counterclockwise', 'Counterclockwise'], ['reverse', WORDS['reverse']!]], start.direction, (v) => set((f) => { f.start.direction = v as typeof start.direction; }))}
    {:else}
      {@render chips('Seam', [['seal', 'Seal'], ['gap', 'Gap'], ['overcut', 'Overcut']], typeof features.seam === 'string' ? 'seal' : 'gap' in features.seam ? 'gap' : 'overcut', (v) => set((f) => { f.seam = v === 'seal' ? 'seal' : v === 'gap' ? { gap: 0.5 } : { overcut: 1 }; }))}
      {#if typeof features.seam !== 'string'}
        {@const seam = features.seam}
        {@render stepper('gap' in seam ? 'Gap' : 'Overcut', 'gap' in seam ? seam.gap : seam.overcut, 'mm', 0.1, (v) => set((f) => { f.seam = 'gap' in seam ? { gap: v } : { overcut: v }; }))}
      {/if}
      <p class="muted" style="font-size:var(--t-sm);margin:0">Gap leaves an attachment; overcut cuts past the start.</p>
    {/if}
  {/if}
</div>
<div class="side-foot">
  <div class="source">{#if draft.feature_source}Prefilled from <strong>{draft.feature_source.name}</strong> ({ago(draft.feature_source.at)}) on {recipeLabel(draft.recipe)}. Saved with this job; the next job on this recipe starts from these.{:else}First job on {recipeLabel(draft.recipe)}. Saved with this job; the next job on this recipe starts from these.{/if}</div>
  <div class="row">
    <button class="btn btn-ghost" onclick={reset}>Reset to defaults</button>
    <button class="btn btn-primary" onclick={() => (ui.setupPanel = null)}>Done</button>
  </div>
</div>
