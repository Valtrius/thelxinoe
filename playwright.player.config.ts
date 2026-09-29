import { defineConfig } from '@playwright/test';
const port = process.env.THELXINOE_PLAYER_PORT ?? '18487';

export default defineConfig({
  testDir: 'tests',
  testMatch: 'player.spec.ts',
  workers: 1,
  use: {
    trace: 'on',
    baseURL: `http://127.0.0.1:${port}`,
    headless: true,
    viewport: { width: 1280, height: 900 },
  },
  webServer: {
    command: `npm run dev:web -- --port ${port} --strictPort`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
  },
  outputDir: 'test-results/player',
  reporter: [
    ['list'],
    ['html', { outputFolder: 'playwright-report/player', open: 'never' }],
    ['json', { outputFile: 'test-results/player/results.json' }],
  ],
});
