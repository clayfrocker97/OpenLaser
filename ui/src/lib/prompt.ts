// Prompts that check what was typed and say why they refuse it.
import { osk } from './osk.svelte';
import { ui } from '../stores/ui.svelte';

/** Tap a number that must be at least `min`; anything less says so and changes nothing. */
export function numberAtLeast(label: string, value: number, unit: string, commit: (value: number) => void, min = 0): void {
  osk.number(label, value, unit, (v) => {
    if (Number.isFinite(v) && v >= min) commit(v);
    else ui.say(`Enter ${min} or more.`, true);
  });
}
