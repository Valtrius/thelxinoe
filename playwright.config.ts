import { defineConfig } from '@playwright/test';
import { browserConnection } from './scripts/ci-browser.mjs';
export default defineConfig({
  testDir: 'tests',
  workers: 1,
  timeout: 60000,
  expect: { timeout: 15000 },
  use: {
    trace: 'retain-on-failure',
    connectOptions: browserConnection(),
    baseURL: process.env.THELXINOE_TEST_URL ?? 'https://localhost:9443',
    headless: true,
    ignoreHTTPSErrors: true,
  },
  projects: [
    { name: 'proxy', testMatch: 'proxy.spec.ts' },
    {
      name: 'authentication',
      testMatch: 'auth-methods.spec.ts',
      dependencies: ['proxy'],
    },
    {
      name: 'catalog',
      testMatch: 'library.spec.ts',
      dependencies: ['proxy'],
    },
  ],
  outputDir: 'test-results/e2e',
  reporter: [
    ['list'],
    ['html', { outputFolder: 'playwright-report/e2e', open: 'never' }],
  ],
});
