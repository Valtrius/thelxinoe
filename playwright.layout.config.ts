import { defineConfig } from '@playwright/test';
const port = process.env.THELXINOE_LAYOUT_PORT ?? '18488';

export default defineConfig({
  testDir: 'tests',
  testMatch: [
    'layout.spec.ts',
    'statistics.spec.ts',
    'provider-setup.spec.ts',
    'settings-details.spec.ts',
    'seerr.spec.ts',
    'service-onboarding.spec.ts',
    'service-updates.spec.ts',
    'server-updates.spec.ts',
    'authentication.spec.ts',
  ],
  workers: 1,
  use: {
    trace: 'on',
    baseURL: `http://127.0.0.1:${port}`,
    headless: true,
    viewport: { width: 1440, height: 1000 },
  },
  webServer: {
    command: `npm run dev:web -- --port ${port} --strictPort`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
  },
  outputDir: 'test-results/layout',
  reporter: [
    ['list'],
    ['html', { outputFolder: 'playwright-report/layout', open: 'never' }],
    ['json', { outputFile: 'test-results/layout/results.json' }],
  ],
});
