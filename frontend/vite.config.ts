import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwind from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
import { offlinePage } from './offline-page.mjs';
export default defineConfig({
  plugins: [svelte(), tailwind(), offlinePage()],
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
  test: { include: ['src/**/*.test.ts'] },
});
