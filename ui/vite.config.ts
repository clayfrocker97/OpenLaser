import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// The dev server proxies the API to the Rust binary, so `npm run dev`
// beside `openlaser --simulate` gives hot reload against the simulator.
export default defineConfig({
  plugins: [svelte()],
  publicDir: 'static',
  build: { outDir: 'dist', emptyOutDir: true, target: 'es2022' },
  server: {
    port: 5173,
    proxy: { '/api': { target: 'http://127.0.0.1:8080', changeOrigin: false } },
  },
});
