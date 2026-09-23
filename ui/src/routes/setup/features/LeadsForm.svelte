<script lang="ts">
  // Lead-ins and lead-outs: the open tab picks which one is edited.
  import Chips from '../../../components/params/Chips.svelte';
  import Stepper from '../../../components/params/Stepper.svelte';
  import Toggle from '../../../components/params/Toggle.svelte';
  import { defaultLeads } from '../../../lib/features';
  import { plural } from '../../../lib/format';
  import type { Features, Lead, Leads } from '../../../api';
  import type { SetFeatures } from './form';

  let { features, on, tab, selectedContours, set }: {
    features: Features;
    on: boolean;
    /** 0 edits the lead-in, 1 the lead-out. */
    tab: number;
    selectedContours: number[];
    set: SetFeatures;
  } = $props();

  const SHAPES = [['line', 'Line'], ['arc', 'Arc'], ['line_arc', 'Line and arc']] as const;
  const SIDES = [['auto', 'Auto'], ['inside', 'Inside'], ['outside', 'Outside']] as const;
  const defaultLead = (): Lead => ({ shape: 'line', length: 2, radius: 1, angle: 45 });

  const leads = $derived(features.leads ?? defaultLeads());
  const lead = $derived(tab === 0 ? leads.entry : leads.exit);
  const selectionEdited = $derived(leads.overrides.some(edited => selectedContours.includes(edited.location.contour)));

  function switchLead(v: boolean): void {
    set((f) => {
      const l = f.leads ?? defaultLeads();
      if (tab === 0) l.entry = v ? defaultLead() : null;
      else l.exit = v ? defaultLead() : null;
      f.leads = l;
    });
  }
  /** Changes the lead on the open tab. */
  function edit(change: (l: Lead) => void): void {
    set((f) => change(tab === 0 ? f.leads!.entry! : f.leads!.exit!));
  }
  const setShape = (v: string) => edit((l) => { l.shape = v as Lead['shape']; });
  const setLength = (v: number) => edit((l) => { l.length = v; });
  const setRadius = (v: number) => edit((l) => { l.radius = v; });
  const setAngle = (v: number) => edit((l) => { l.angle = v; });
  const setSide = (v: string) => set((f) => { f.leads!.side = v as Leads['side']; });
  const setClosedOnly = (v: boolean) => set((f) => { f.leads!.closed_only = v; });
  function useDefaultsForSelection(): void {
    set(f => { f.leads!.overrides = f.leads!.overrides.filter(edited => !selectedContours.includes(edited.location.contour)); });
  }
  function resetOverrides(): void {
    set(f => { f.leads!.overrides = []; });
  }
</script>

<Toggle label={tab === 0 ? 'Lead-in' : 'Lead-out'} value={lead !== null} disabled={!on} apply={switchLead} />
{#if lead}
  <Chips label="Shape" options={SHAPES} current={lead.shape} disabled={!on} apply={setShape} />
  {#if lead.shape !== 'arc'}<Stepper label="Length" value={lead.length} unit="mm" step={0.5} disabled={!on} apply={setLength} />{/if}
  {#if lead.shape !== 'line'}<Stepper label="Arc radius" value={lead.radius} unit="mm" step={0.5} disabled={!on} apply={setRadius} />{/if}
  <Stepper label={lead.shape === 'arc' ? 'Arc sweep' : 'Angle'} value={lead.angle} unit="°" step={15} disabled={!on} apply={setAngle} />
{/if}
<Chips label="Side" options={SIDES} current={leads.side} disabled={!on} apply={setSide} />
<Toggle label="Closed contours only" value={leads.closed_only} disabled={!on} apply={setClosedOnly} />
{#if leads.overrides?.length}
  <p class="muted" style="font-size:var(--t-sm)">{plural(leads.overrides.length, 'contour')} with individual lead settings</p>
  <button class="btn btn-soft" disabled={!on || !selectionEdited} onclick={useDefaultsForSelection}>Use job defaults for selection</button>
  <button class="btn btn-ghost" disabled={!on} onclick={resetOverrides}>Reset all individual leads</button>
{/if}
