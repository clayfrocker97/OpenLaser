import { mergeConfig } from 'vite';
import app from '../vite.config.ts';
export default mergeConfig(app, {
  plugins: [{ name: 'font-import-acceptance', transformIndexHtml: { order: 'pre',
    handler: () => [{ tag: 'script', attrs: { type: 'module', src: '/bench/fonts.mjs' }, injectTo: 'body' }],
  } }],
});
