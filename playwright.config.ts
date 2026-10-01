import { defineConfig } from '@playwright/test';
import { browserConnection } from './scripts/ci-browser.mjs';
export default defineConfig({
  testDir: 'tests',
  workers: 1,
  timeout: 60000,
  expect: { timeout: 15000 },
  use: {
    trace: 'on',
    connectOptions: browserConnection(),
    baseURL: process.env.THELXINOE_TEST_URL ?? 'http://127.0.0.1:8484',
    headless: true,
    ignoreHTTPSErrors: !!process.env.THELXINOE_PROXY_TEST,
  },
  projects: process.env.THELXINOE_INTERACTION_TEST
    ? [{ name: 'interactions', testMatch: 'interactions.spec.ts' }]
    : process.env.THELXINOE_UI_TEST
      ? [{ name: 'appearance', testMatch: 'appearance.spec.ts' }]
      : process.env.THELXINOE_PROXY_TEST
        ? [
            { name: 'proxy', testMatch: 'proxy.spec.ts' },
            {
              name: 'catalog',
              testMatch: 'library.spec.ts',
              dependencies: ['proxy'],
            },
          ]
        : [{ name: 'setup', testMatch: 'setup.spec.ts' }],
  outputDir: 'test-results/e2e',
  reporter: [
    ['list'],
    ['html', { outputFolder: 'playwright-report/e2e', open: 'never' }],
    ['json', { outputFile: 'test-results/e2e/results.json' }],
  ],
});
