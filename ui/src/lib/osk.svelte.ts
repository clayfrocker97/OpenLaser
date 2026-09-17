// The on-screen keyboards: a text keyboard for any input and a numpad for
// tapped values. One instance, opened with a callback.
import { inputValue, sourceInput, unitLabel, units } from './units.svelte';
export type Kind = 'text' | 'num';

class Osk {
  open = $state(false);
  kind = $state<Kind>('text');
  label = $state('');
  unit = $state('');
  value = $state('');
  shift = $state(false);
  multiline = $state(false);
  /** Whether the next digit replaces the value shown, as a tapped value opens. */
  fresh = $state(false);
  target: HTMLInputElement | HTMLTextAreaElement | null = null;
  onCommit: ((value: string) => void) | null = null;

  show(options: { kind: Kind; label: string; value?: string; unit?: string; fresh?: boolean; target?: HTMLInputElement | HTMLTextAreaElement | null; onCommit?: (value: string) => void }): void {
    this.kind = options.kind;
    this.label = options.label;
    this.unit = options.unit ?? '';
    this.value = options.value ?? '';
    this.fresh = options.fresh ?? false;
    this.shift = false;
    this.target = options.target ?? null;
    this.multiline = this.target instanceof HTMLTextAreaElement;
    this.onCommit = options.onCommit ?? null;
    this.open = true;
  }

  /** Tap a number to type it. */
  number(label: string, value: number, unit: string, onCommit: (value: number) => void): void {
    const system = units.system;
    this.show({ kind: 'num', label, value: inputValue(value, unit, system), unit: unitLabel(unit, system), fresh: true, onCommit: (v) => { const n = sourceInput(v, value, unit, system); if (Number.isFinite(n)) onCommit(n); } });
  }

  text(label: string, value: string, onCommit: (value: string) => void): void {
    this.show({ kind: 'text', label, value, onCommit });
  }

  key(k: string): void {
    if (k === 'done') { const v = this.value, cb = this.onCommit; this.close(); if (cb) cb(v); return; }
    if (k === 'shift') { this.shift = !this.shift; return; }
    if (this.kind === 'text' && this.target) { this.editText(k); return; }
    if (this.fresh) { this.value = k === '−' ? this.value : ''; this.fresh = false; }
    if (k === 'bs') this.value = this.value.slice(0, -1);
    else if (k === 'clear') this.value = '';
    else if (k === '−') this.value = this.value.startsWith('-') ? this.value.slice(1) : '-' + this.value;
    else if (k === '.' && this.kind === 'num') { if (!this.value.includes('.')) this.value += '.'; }
    else { this.value += this.shift ? k.toUpperCase() : k; if (this.shift && this.kind === 'text') this.shift = false; }
    if (this.target) { this.target.value = this.value; this.target.dispatchEvent(new Event('input', { bubbles: true })); }
  }

  /** Touch keys replace the selection or insert at the text caret. */
  private editText(key: string): void {
    const target = this.target!;
    let start = target.selectionStart ?? target.value.length, end = target.selectionEnd ?? start;
    let value = this.shift ? key.toUpperCase() : key;
    if (key === 'clear') { start = 0; end = target.value.length; value = ''; }
    else if (key === 'bs') {
      if (start === end) start -= [...target.value.slice(0, start)].at(-1)?.length ?? 0;
      value = '';
    } else if (this.shift) this.shift = false;
    this.value = target.value.slice(0, start) + value + target.value.slice(end);
    target.value = this.value;
    target.focus({ preventScroll: true });
    if (target.selectionStart !== null) target.setSelectionRange(start + value.length, start + value.length);
    target.dispatchEvent(new Event('input', { bubbles: true }));
  }

  close(): void {
    this.open = false;
    if (this.target) this.target.blur();
    this.target = null;
    this.multiline = false;
    this.onCommit = null;
  }
}

export const osk = new Osk();
