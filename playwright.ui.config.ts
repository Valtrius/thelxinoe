import { defineConfig } from '@playwright/test';
import { freePort } from './scripts/ci-resources.mjs';
const port = (process.env.THELXINOE_UI_PORT ??= String(await freePort()));
const playerPort = (process.env.THELXINOE_PLAYER_PORT ??= String(
  await freePort(),
));

export default defineConfig({
  testDir: 'tests',
  projects: [
    {
      name: 'layout',
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
        'desktop-startup.spec.ts',
      ],
    },
    {
      name: 'player',
      testMatch: 'player.spec.ts',
      use: {
        baseURL: `http://127.0.0.1:${playerPort}`,
        viewport: { width: 1280, height: 900 },
      },
    },
  ],
  timeout: 60000,
  expect: { timeout: 15000 },
  workers: 1,
  use: {
    trace: 'on',
    baseURL: `http://127.0.0.1:${port}`,
    headless: true,
    viewport: { width: 1440, height: 1000 },
  },
  webServer: [
    {
      command: `pnpm --dir frontend exec vite preview --host 127.0.0.1 --port ${port} --strictPort`,
      url: `http://127.0.0.1:${port}`,
      reuseExistingServer: false,
    },
    {
      command: `pnpm run dev:web --port ${playerPort} --strictPort`,
      url: `http://127.0.0.1:${playerPort}`,
      reuseExistingServer: false,
    },
  ],
  outputDir: 'test-results/ui',
  reporter: [
    ['list'],
    ['html', { outputFolder: 'playwright-report/ui', open: 'never' }],
    ['json', { outputFile: 'test-results/ui/results.json' }],
  ],
});
