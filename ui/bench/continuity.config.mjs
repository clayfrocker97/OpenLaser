import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { mergeConfig } from 'vite';
import app from '../vite.config.ts';

// Optional snapshots let the same checks demonstrate the original blank
// frames without changing the working source. Neither runner is shipped.
const baseline = process.env.CONTINUITY_BASELINE_SOURCE;
const snapshots = ['src/stores/server.svelte.ts', 'src/routes/setup/Canvas.svelte'];
export default mergeConfig(app, {
  define: { __CONTINUITY_BASELINE__: JSON.stringify(!!baseline) },
  plugins: [{
    name: 'preview-continuity-checks', enforce: 'pre',
    load(id) {
      const file = snapshots.find(file => id === resolve(file));
      if (baseline && file) return readFileSync(resolve(baseline, 'ui', file), 'utf8');
    },
    transformIndexHtml: { order: 'pre', handler: () => [
      { tag: 'script', attrs: { type: 'module', src: '/bench/continuity.mjs' }, injectTo: 'body' },
    ] },
  }],
});
