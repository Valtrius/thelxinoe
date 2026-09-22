import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: 'tests',
  testMatch: ['layout.spec.ts', 'statistics.spec.ts'],
  workers: 1,
  use: {
    baseURL: 'http://127.0.0.1:18488',
    headless: true,
    viewport: { width: 1440, height: 1000 },
  },
  webServer: {
    command: 'npm run dev:web -- --port 18488',
    url: 'http://127.0.0.1:18488',
    reuseExistingServer: false,
  },
  reporter: 'list',
});
