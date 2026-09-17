import { mergeConfig } from 'vite';
import app from '../vite.config.ts';

// The production build never imports the benchmark runner.
export default mergeConfig(app, {
  plugins: [{
    name: 'openlaser-viewer-benchmark',
    transformIndexHtml: {
      order: 'pre',
      handler: () => [{ tag: 'script', attrs: { type: 'module', src: '/bench/main.mjs' }, injectTo: 'body' }],
    },
  }],
});
