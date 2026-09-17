import { mergeConfig } from 'vite';
import app from '../vite.config.ts';

export default mergeConfig(app, {
  plugins: [{ name: 'individual-lead-checks', transformIndexHtml: { order: 'pre',
    handler: () => [{ tag: 'script', attrs: { type: 'module', src: '/bench/leads.mjs' }, injectTo: 'body' }],
  } }],
});
