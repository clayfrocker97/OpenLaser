import { describe, expect, it } from 'vitest';
import type { Document } from '../api';
import { nextStep } from './next-step';

type State = { program?: string; recovery?: string; resumable?: boolean; connected?: boolean; age?: number; homed?: boolean; head?: boolean; headHomed?: boolean; calibrated?: boolean; quality?: string | null; draft?: boolean; captured?: boolean; mode?: 'head' | 'fixed' };

// Only the fields nextStep reads; the rest of the document is irrelevant here.
function doc(s: State = {}): Document {
  const connected = s.connected ?? true;
  return {
    machine: {
      connection: connected ? { state: 'connected' } : { state: 'disconnected' },
      feedback: connected ? { age_ms: s.age ?? 50, referenced: [s.homed ?? true, s.homed ?? true], head: { referenced: s.headHomed ?? true } } : null,
      session: { homed: s.homed ?? true },
      program: s.program ? { state: s.program } : null,
    },
    bindings: { head_enabled: s.head ?? true },
    can_resume: s.resumable ?? false,
    recovery: s.recovery ? { state: s.recovery } : null,
    calibration: { current: s.calibrated ?? true, quality: s.quality ?? null },
    draft: s.draft === false ? null : { placement: { mode: s.mode ?? 'head', captured: s.captured ?? true, correction_pending: false, saved: false } },
  } as unknown as Document;
}

describe('nextStep', () => {
  it('walks connect, home XY, home Z, calibrate Z, set origin, in order', () => {
    expect(nextStep(doc({ connected: false }))).toBe('Next: connect');
    expect(nextStep(doc({ age: 5000 }))).toBe('Waiting for the machine');
    expect(nextStep(doc({ homed: false, headHomed: false, calibrated: false, captured: false }))).toBe('Next: home XY');
    expect(nextStep(doc({ headHomed: false, calibrated: false, captured: false }))).toBe('Next: home Z');
    expect(nextStep(doc({ calibrated: false, captured: false }))).toBe('Next: calibrate Z');
    expect(nextStep(doc({ calibrated: false, quality: 'material changed' }))).toBe('Next: calibrate Z (material changed)');
    expect(nextStep(doc({ captured: false }))).toBe('Next: set origin');
    expect(nextStep(doc())).toBeNull();
  });

  it('skips the origin when a loaded job carries an absolute one', () => {
    expect(nextStep(doc({ mode: 'fixed', captured: true }))).toBeNull();
    expect(nextStep(doc({ mode: 'fixed', captured: false }))).toBe('Next: set origin');
  });

  it('stays quiet while a program runs or is paused', () => {
    expect(nextStep(doc({ captured: false, program: 'running' }))).toBeNull();
    expect(nextStep(doc({ captured: false, program: 'held' }))).toBeNull();
    expect(nextStep(doc({ captured: false, program: 'completed' }))).toBe('Next: set origin');
  });

  it('does not ask for an origin while a stopped run can resume', () => {
    expect(nextStep(doc({ captured: false, resumable: true }))).toBeNull();
    expect(nextStep(doc({ captured: false, recovery: 'stopped' }))).toBeNull();
    expect(nextStep(doc({ captured: false, recovery: 'completed' }))).toBe('Next: set origin');
  });

  it('skips the head steps without a head controller, and the origin without a job', () => {
    expect(nextStep(doc({ head: false, headHomed: false, calibrated: false, captured: false }))).toBe('Next: set origin');
    expect(nextStep(doc({ draft: false }))).toBeNull();
  });
});
