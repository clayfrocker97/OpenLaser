import { mergeConfig } from 'vite';
import app from '../vite.config.ts';

export default mergeConfig(app, {
  plugins: [{ name: 'checklist-acceptance', transformIndexHtml: { order: 'pre',
    handler: () => ['checklists', 'copy-paste'].map(name => ({ tag: 'script', attrs: { type: 'module', src: `/bench/${name}.mjs` }, injectTo: 'body' })),
  } }],
});
