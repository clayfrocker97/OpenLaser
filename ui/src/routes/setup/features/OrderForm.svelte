<script lang="ts">
  // The cutting order: a strategy, or contours tapped or typed in order.
  import Chips from '../../../components/params/Chips.svelte';
  import Toggle from '../../../components/params/Toggle.svelte';
  import { WORDS } from '../../../lib/features';
  import { osk } from '../../../lib/osk.svelte';
  import { ui } from '../../../stores/ui.svelte';
  import type { Features, Preview } from '../../../api';
  import type { PickFeature, SetFeatures } from './form';

  let { features, on, preview, set, pick }: {
    features: Features;
    on: boolean;
    /** The drawing's preview, for the contours an order must list. */
    preview: Preview | null;
    set: SetFeatures;
    pick: (feature: PickFeature) => void;
  } = $props();

  type Strategy = Exclude<Features['order']['strategy'], object>;
  const STRATEGIES: ReadonlyArray<readonly [string, string]> = [
    ...(['as_drawn', 'nearest', 'left_to_right', 'right_to_left', 'bottom_to_top', 'top_to_bottom'] as const)
      .map((s) => [s, WORDS[s]!] as const),
    ['manual', 'Picked'],
  ];

  const order = $derived(features.order);
  const strategy = $derived(typeof order.strategy === 'string' ? order.strategy : 'manual');

  function setStrategy(v: string): void {
    if (v === 'manual') pick('order');
    else set((f) => { f.order.strategy = v as Strategy; });
  }
  function typedOrder(): void {
    const current = order.strategy;
    const typed = typeof current === 'object' ? current.manual.map(v => v + 1).join(', ') : '';
    osk.text('Contour IDs in cutting order (starting at 1)', typed, (text) => {
      const order = text.split(',').map(v => v.trim() ? Number(v.trim()) - 1 : NaN);
      const all = [...new Set(preview?.contours.flatMap(c => c.sources) ?? [])];
      const valid = order.length === all.length && new Set(order).size === order.length
        && order.every(v => Number.isInteger(v) && all.includes(v));
      if (!valid) { ui.say(`Enter each active contour ID once: ${all.map(v => v + 1).join(', ')}.`, true); return; }
      set(f => { f.order.strategy = { manual: order }; });
    });
  }
  const setInnerFirst = (v: boolean) => set((f) => { f.order.inner_first = v; });
  const setCirclesFirst = (v: boolean) => set((f) => { f.order.circles_first = v; });
  const setSpreadHeat = (v: boolean) => set((f) => { f.order.spread_heat = v; });
</script>

<Chips label="Strategy" options={STRATEGIES} current={strategy} disabled={!on} apply={setStrategy} />
{#if typeof order.strategy !== 'string'}
  <div class="param">
    <div class="lbl">Contours cut in the order they were tapped</div>
    <button class="btn btn-soft" onclick={() => pick('order')}>Pick again</button>
  </div>
{/if}
<button class="btn btn-soft" onclick={typedOrder}>Enter contour order…</button>
<Toggle label="Inner contours first" value={order.inner_first} disabled={!on} apply={setInnerFirst} />
<Toggle label="Circles first" value={order.circles_first} disabled={!on} apply={setCirclesFirst} />
<Toggle label="Spread heat" value={order.spread_heat} disabled={!on} apply={setSpreadHeat} />
