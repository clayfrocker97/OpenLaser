// The one thing to do next before cutting, in the order it is usually done:
// connect, load the machine backup, home XY, home Z, then calibrate Z. Null when the machine is ready.
import type { Document } from '../api';

/** Feedback older than this is not trusted for readiness (twenty 50 ms polls). */
const FRESH_FEEDBACK_MS = 1000;

export function nextStep(doc: Document): string | null {
  const machine = doc.machine;
  if (machine.connection.state !== 'connected') return 'Next: connect';
  if (!doc.bindings) return 'Next: load the machine backup';
  const feedback = machine.feedback;
  if (!feedback || feedback.age_ms > FRESH_FEEDBACK_MS) return 'Waiting for the machine';
  if (!machine.session.homed || !feedback.referenced.every(Boolean)) return 'Next: home XY';
  const head = doc.bindings?.head_enabled ?? false;
  if (head && !feedback.head.referenced) return 'Next: home Z';
  if (head && !doc.calibration.current) return doc.calibration.quality ? 'Next: calibrate Z (material changed)' : 'Next: calibrate Z';
  return null;
}
