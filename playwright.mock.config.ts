import { defineConfig } from '@playwright/test';
import { browserConnection } from './scripts/ci-browser.mjs';
import { freePort } from './scripts/ci-resources.mjs';

const port = (process.env.THELXINOE_MOCK_PORT ??= String(await freePort()));
export default defineConfig({
  testDir: 'tests',
  testMatch: [
    'mock-preview.spec.ts',
    'mock-service-scenarios.spec.ts',
    'mock-ownership-motion.spec.ts',
    'page-transition.spec.ts',
    'animation-duration.spec.ts',
    'mobile-media.spec.ts',
    'mobile-player.spec.ts',
  ],
  timeout: 120000,
  expect: { timeout: 15000 },
  workers: 1,
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    connectOptions: browserConnection(),
    viewport: { width: 1440, height: 1000 },
    actionTimeout: 15000,
    navigationTimeout: 15000,
    trace: 'on',
  },
  webServer: {
    command: 'node scripts/dev-mock.mjs',
    url: `http://127.0.0.1:${port}/__mock/health`,
    reuseExistingServer: false,
    timeout: 120000,
  },
  outputDir: 'test-results/mock',
  reporter: [
    ['list'],
    ['html', { outputFolder: 'playwright-report/mock', open: 'never' }],
    ['json', { outputFile: 'test-results/mock/results.json' }],
  ],
});
