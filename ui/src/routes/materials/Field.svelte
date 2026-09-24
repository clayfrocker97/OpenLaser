<script lang="ts">
  import { unitLabel } from '../../lib/units.svelte';
  // One control of the recipe: a number or text box that opens the
  // keypad, a flag that flips, or the gas selections as chips. A pressure
  // whose gas has no proportional output is set at the regulator, and a
  // gas selection the machine has no valve for is shown as unwired.
  import { GAS, bare, field, isOn, pairedGas, type Editor } from '../../lib/recipe';

  let { ed, key, label, big = false, row = false, plain = false, note, value, oncommit }: {
    ed: Editor;
    key: string;
    /** A label other than the field's own. */
    label?: string;
    /** A large control, as the main pages show. */
    big?: boolean;
    /** A flag as a row with its switch at the end, as the sheets show. */
    row?: boolean;
    /** The gas chips alone, without a label.  */
    plain?: boolean;
    note?: string;
    /** A value other than the key's own, for a control over several keys. */
    value?: string;
    oncommit?: (value: string) => void;
  } = $props();

  const f = $derived(field(key));
  const v = $derived(value ?? ed.values[key] ?? '');
  const changed = $derived(key in ed.edits);
  const wired = (selector: number) => ed.a.gases.includes(selector);
  /** The wired selections, plus the recipe's own when it is not wired. */
  const choices = $derived([...new Set([...ed.a.gases, ...(v !== '' && Number.isFinite(Number(v)) ? [Number(v)] : [])])].sort((a, b) => a - b));
  const gasKey = $derived(pairedGas(key));
  const regulator = $derived(gasKey !== null && ed.a.gas && !ed.a.pressure(Number(ed.values[gasKey] ?? -1)));
  const tap = () => ed.tap(key, oncommit ? { value: v, commit: oncommit } : undefined);
</script>

{#snippet chips()}
  {#if ed.a.fixedGas !== null}
    <div class="box static"><b>{GAS[ed.a.fixedGas]}</b>{#if !wired(ed.a.fixedGas)}<span class="unit">valve not assigned</span>{/if}</div>
  {:else}
  <div class="chips">{#each choices as s (s)}<button class="chip" class:on={v === String(s)} class:warn={!wired(s)} onclick={() => ed.set(key, String(s))}>{wired(s) ? GAS[s] ?? `Selection ${s}` : `Unwired ${s}`}</button>{/each}</div>
  {/if}
{/snippet}

{#if f.kind === 'gas' && plain}
  {@render chips()}
{:else if f.kind === 'flag' && row}
  <div class="toggle-row" class:changed>
    <div><strong>{label ?? f.label}</strong>{#if note ?? f.explain}<small>{note ?? f.explain}</small>{/if}</div>
    <button class="switch" class:on={isOn(v)} onclick={tap} aria-label={label ?? f.label}></button>
  </div>
{:else}
  <div class="fld" class:changed class:big>
    <span class="lbl">{label ?? f.label}{#if f.explain && !note}<small class="explain">{f.explain}</small>{/if}</span>
    {#if f.kind === 'gas'}
      {@render chips()}
    {:else if regulator}
      <div class="box static"><b>Set at regulator</b>{#if v !== ''}<span class="unit">file says {bare(key, v)} {unitLabel(f.unit ?? '')}</span>{/if}</div>
    {:else}
      <button class="box" class:on={ed.editing === key} data-numpad={f.kind !== 'flag' || undefined} onclick={tap}>
        <b>{bare(key, v)}</b>
        {#if f.kind === 'flag'}<span class="switch small" class:on={isOn(v)}></span>{:else if f.unit}<span class="unit">{unitLabel(f.unit ?? '')}</span>{/if}
      </button>
    {/if}
    {#if note}<small class="muted">{note}</small>{/if}
  </div>
{/if}
