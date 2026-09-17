import { describe, expect, it, vi } from 'vitest';
import type { PreflightPreferences } from '../api';
vi.mock('../stores/ui.svelte', () => ({ ui: { previewTheme: vi.fn() } }));
import { mergePreferences, withPostflight } from './settings-edits.svelte';

const preferences = (): PreflightPreferences => ({
  version: 2, revision: 3, confirm_gas: false,
  fiber: { enabled: true, steps: [{ text: 'Check sheet', action: null, auto_check: false }] },
  co2: { enabled: true, steps: [] },
  pause: {
    fiber: { enabled: true, steps: [{ text: 'Clear debris', action: null, auto_check: false }] },
    co2: { enabled: true, steps: [] },
  },
  postflight: {
    fiber: { enabled: true, steps: [{ text: 'Park', action: { kind: 'move_xy', x: 10, y: 20 }, auto_check: true }] },
    co2: { enabled: true, steps: [] },
  },
});

describe('shared checklist drafts', () => {
  it('merges independent preflight and postflight edits without losing either', () => {
    const base = preferences(), draft = preferences(), saved = preferences();
    draft.postflight.fiber.steps[0]!.text = 'Unload';
    draft.pause.fiber.steps[0]!.text = 'Check nozzle';
    saved.fiber.steps[0]!.text = 'Check new sheet';
    saved.revision++;
    const result = mergePreferences(base, draft, saved);
    expect(result.conflicts).toEqual([]);
    expect(result.value.fiber.steps[0]!.text).toBe('Check new sheet');
    expect(result.value.postflight.fiber.steps[0]!.text).toBe('Unload');
    expect(result.value.pause.fiber.steps[0]!.text).toBe('Check nozzle');
    expect(result.value.revision).toBe(saved.revision);
    expect(base.postflight.fiber.steps[0]!.text).toBe('Park');
  });

  it('identifies conflicting postflight edits for review', () => {
    const base = preferences(), draft = preferences(), saved = preferences();
    draft.postflight.fiber.steps[0]!.text = 'Local park';
    saved.postflight.fiber.steps[0]!.text = 'Remote park';
    expect(mergePreferences(base, draft, saved).conflicts.map(c => c.key)).toEqual(['postflight.fiber.steps']);
  });

  it('backfills old browser drafts without overwriting new postflight defaults', () => {
    const saved = preferences(), old = preferences();
    old.version = 1;
    old.confirm_gas = true;
    saved.fiber.steps.push({ text: 'Gas supply is ready', action: { kind: 'job_gas_test', duration_ms: 500 }, auto_check: false });
    saved.co2.steps.push({ text: 'Gas supply is ready', action: { kind: 'job_gas_test', duration_ms: 500 }, auto_check: false });
    delete (old as Partial<PreflightPreferences>).postflight;
    delete (old as Partial<PreflightPreferences>).pause;
    const draft = structuredClone(old);
    draft.confirm_gas = false;
    const result = mergePreferences(old, draft, saved);
    expect(result.conflicts).toEqual([]);
    expect(result.value.postflight).toEqual(saved.postflight);
    expect(result.value.pause).toEqual(saved.pause);
    expect(result.value.confirm_gas).toBe(false);
    expect(result.value.fiber.steps).toHaveLength(1);
    expect(result.value.co2.steps).toHaveLength(0);
    expect(result.value.version).toBe(2);
    const copy = withPostflight(old, saved);
    copy.postflight.fiber.steps[0]!.text = 'Different';
    expect(saved.postflight.fiber.steps[0]!.text).toBe('Park');
    expect(copy.fiber.steps.filter(step => step.action?.kind === 'job_gas_test')).toHaveLength(1);
  });
});
