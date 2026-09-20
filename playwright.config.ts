import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: 'tests',
  use: {
    baseURL: process.env.THELXINOE_TEST_URL ?? 'http://127.0.0.1:8484',
    headless: true,
    ignoreHTTPSErrors: !!process.env.THELXINOE_PROXY_TEST,
  },
  projects: process.env.THELXINOE_UI_TEST
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
  reporter: 'list',
});
