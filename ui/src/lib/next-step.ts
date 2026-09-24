// The one thing to do next before cutting, in the order it is usually done:
// connect, home XY, home Z, calibrate Z, then set the origin. Alarms are left
// to the bell. A job with a saved absolute origin already has one, so it
// skips that step; an each-run origin is set again for every run. Null when
// the machine is ready.
import type { Document } from '../api';

/** Feedback older than this is not trusted for readiness (twenty 50 ms polls). */
const FRESH_FEEDBACK_MS = 1000;

export function nextStep(doc: Document): string | null {
  const machine = doc.machine;
  if (machine.connection.state !== 'connected') return 'Next: connect';
  // While a program runs or is paused, the run's own messages say what is happening.
  if (['running', 'finishing', 'held'].includes(machine.program?.state ?? '')) return null;
  const feedback = machine.feedback;
  if (!feedback || feedback.age_ms > FRESH_FEEDBACK_MS) return 'Waiting for the machine';
  if (!machine.session.homed || !feedback.referenced.every(Boolean)) return 'Next: home XY';
  const head = doc.bindings?.head_enabled ?? false;
  if (head && !feedback.head.referenced) return 'Next: home Z';
  // Start refuses until Z is calibrated for this material at this W table height.
  if (head && !doc.calibration.current) return doc.calibration.stale ? `Next: calibrate Z (${doc.calibration.stale})` : 'Next: calibrate Z';
  // Resuming keeps the stopped run's origin, so only a new run needs one.
  const resuming = doc.can_resume || ['held', 'stopped', 'failed'].includes(doc.recovery?.state ?? '');
  if (doc.draft && !doc.draft.placement.captured && !resuming) return 'Next: set origin';
  return null;
}
