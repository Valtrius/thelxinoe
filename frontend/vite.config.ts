import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwind from '@tailwindcss/vite';
export default defineConfig({
  plugins: [svelte(), tailwind()],
  server: {
    strictPort: true,
    proxy: { '/api': { target: 'http://127.0.0.1:8484', ws: true } },
  },
  test: { include: ['src/**/*.test.ts'] },
});
