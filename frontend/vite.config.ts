import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwind from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
import { offlinePage } from './offline-page.mjs';
import { apiVersion } from './api-version.mjs';
export default defineConfig({
  plugins: [svelte(), tailwind(), offlinePage()],
  define: { __THELXINOE_API_VERSION__: JSON.stringify(apiVersion()) },
  resolve: {
    alias: {
      // jassub's requestVideoFrameCallback polyfill is GPL-3.0.
      'rvfc-polyfill': fileURLToPath(
        new URL('./src/lib/no-polyfill.ts', import.meta.url),
      ),
    },
  },
  // The styled subtitle renderer runs in a module worker.
  worker: { format: 'es' },
  build: {
    rolldownOptions: {
      input: {
        app: fileURLToPath(new URL('./index.html', import.meta.url)),
        splashscreen: fileURLToPath(
          new URL('./splashscreen.html', import.meta.url),
        ),
        offline: fileURLToPath(new URL('./offline.html', import.meta.url)),
        about: fileURLToPath(new URL('./about/index.html', import.meta.url)),
        privacy: fileURLToPath(
          new URL('./about/privacy.html', import.meta.url),
        ),
      },
    },
  },
  server: {
    strictPort: true,
    proxy: {
      '/api': { target: 'http://127.0.0.1:8484', ws: true },
      '/services': { target: 'http://127.0.0.1:8484', ws: true },
      '/service-access': { target: 'http://127.0.0.1:8484', ws: true },
    },
  },
  // responsive.test.ts reads app.css, which Vitest otherwise replaces with an empty module.
  test: { include: ['src/**/*.test.ts'], css: { include: /app\.css/ } },
});
