import { api } from '../api/client';
import { FeatureEdits } from '../lib/feature-edits.svelte';
import { server } from './server.svelte';

export const featureEdits = new FeatureEdits(() => server.doc?.draft ?? null, async (features, revision) => {
  const reply = await api.setFeatures(features, revision);
  if (!reply.draft) throw new Error('The draft was closed.');
  return reply.draft;
});
