import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: 'tests',
  testMatch: 'player.spec.ts',
  workers: 1,
  use: {
    baseURL: 'http://127.0.0.1:18487',
    headless: true,
    viewport: { width: 1280, height: 900 },
  },
  webServer: {
    command: 'npm run dev --workspace frontend -- --port 18487',
    url: 'http://127.0.0.1:18487',
    reuseExistingServer: false,
  },
  reporter: 'list',
});
