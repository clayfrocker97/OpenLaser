import type { PreflightIntent, PreflightReview } from '../api';
import { api } from '../api/client';

interface RunSnapshot {
  execution: { frame:boolean } | null;
  machine: { program:{ state:string } | null };
}

/** Both layouts enter the same positioning and preflight workflow. */
export async function beginRun(intent: PreflightIntent, source: RunSnapshot): Promise<PreflightReview | null> {
  if (intent === 'run') {
    // Start after an ended job prepares the next sheet before reviewing it.
    if (source.execution && !source.execution.frame && ['completed', 'stopped'].includes(source.machine.program?.state ?? '')) await api.placement({ kind:'new_run' });
    await api.preparePlacement();
  }
  const review = await api.preflight(intent);
  if (review.steps.length || review.confirm_gas) return review;
  await api.machine(intent, { preflight: { token: review.token, checked: [], gas_ready: false } });
  return null;
}
