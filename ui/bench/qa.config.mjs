import { mergeConfig } from 'vite';
import app from '../vite.config.ts';

export default mergeConfig(app, {
  plugins: [{ name: 'viewer-gesture-checks', transformIndexHtml: { order: 'pre',
    handler: () => [{ tag: 'script', attrs: { type: 'module', src: '/bench/gestures.mjs' }, injectTo: 'body' }],
  } }],
});
