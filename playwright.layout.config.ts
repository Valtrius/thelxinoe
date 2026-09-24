import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: 'tests',
  testMatch: [
    'layout.spec.ts',
    'statistics.spec.ts',
    'provider-setup.spec.ts',
    'settings-details.spec.ts',
    'seerr.spec.ts',
  ],
  workers: 1,
  use: {
    trace: 'on',
    baseURL: 'http://127.0.0.1:18488',
    headless: true,
    viewport: { width: 1440, height: 1000 },
  },
  webServer: {
    command: 'npm run dev:web -- --port 18488',
    url: 'http://127.0.0.1:18488',
    reuseExistingServer: false,
  },
  outputDir: 'test-results/layout',
  reporter: [
    ['list'],
    ['html', { outputFolder: 'playwright-report/layout', open: 'never' }],
    ['json', { outputFile: 'test-results/layout/results.json' }],
  ],
});
