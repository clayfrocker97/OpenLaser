import type { Document, LinkPhase } from '../api';
import type { ConfirmRequest } from '../stores/ui.svelte';

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

/** Disconnecting stops whatever runs first: ask before it ends a job or an operation. */
export async function confirmDisconnect(doc: Document | null, ask: (request: ConfirmRequest) => Promise<boolean>): Promise<boolean> {
  const program = doc?.machine.program?.state;
  if (!doc?.machine.operation && !['running', 'finishing', 'held'].includes(program ?? '')) return true;
  return ask({
    title: 'Disconnect the machine?',
    body: program === 'held'
      ? 'The paused job ends: the machine stops and the job can only restart from recovery.'
      : 'The machine stops what it is doing before the link closes.',
    confirm: 'Stop and disconnect',
    danger: true,
  });
}
