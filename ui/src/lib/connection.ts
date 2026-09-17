import type { Document, LinkPhase } from '../api';

const STEPS: Record<LinkPhase, string> = {
  idle: 'Connect', failed: 'Connect', inspecting: 'Finding the cable…',
  configuring: 'Setting the address…', connecting: 'Reaching the controller…',
  reading: 'Reading the machine…',
};

/** One connection status label for every interface. */
export function connectionLabel(doc: Document | null): string {
  if (doc?.machine.connection.state === 'connected') return 'Connected';
  if (doc && doc.link.phase !== 'idle' && doc.link.phase !== 'failed') return STEPS[doc.link.phase];
  return doc?.machine.connection.state === 'faulted' ? 'Reconnect' : 'Connect';
}
