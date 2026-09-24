import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: 'tests',
  testMatch: 'player.spec.ts',
  workers: 1,
  use: {
    trace: 'on',
    baseURL: 'http://127.0.0.1:18487',
    headless: true,
    viewport: { width: 1280, height: 900 },
  },
  webServer: {
    command: 'npm run dev:web -- --port 18487',
    url: 'http://127.0.0.1:18487',
    reuseExistingServer: false,
  },
  outputDir: 'test-results/player',
  reporter: [
    ['list'],
    ['html', { outputFolder: 'playwright-report/player', open: 'never' }],
    ['json', { outputFile: 'test-results/player/results.json' }],
  ],
});
