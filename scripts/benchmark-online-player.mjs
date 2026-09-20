// Run only against an explicitly selected development instance with linked Twitch.
// Credentials stay in memory; results contain timings, not stream addresses.
import { chromium } from '@playwright/test';
import { writeFileSync } from 'node:fs';

const origin = process.env.THELXINOE_BENCHMARK_URL;
if (!origin)
  throw new Error('Set THELXINOE_BENCHMARK_URL to the development server');
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
const results = [];
const output =
  process.env.THELXINOE_BENCHMARK_OUTPUT ??
  '.local/startup-benchmark/player.json';
const headers = { 'X-Thelxinoe-Client': '1' };
const api = async (path, method = 'GET', data) => {
  const response = await context.request.fetch(`${origin}/api/v1${path}`, {
    method,
    data,
    headers,
  });
  if (!response.ok())
    throw new Error(`${path.split('?')[0]}: HTTP ${response.status()}`);
  return response.json();
};
try {
  await api('/auth/login', 'POST', {
    username: process.env.THELXINOE_BENCHMARK_USER ?? 'admin',
    password:
      process.env.THELXINOE_BENCHMARK_PASSWORD ?? 'test-only long passphrase',
  });
  const feed = await api('/online/twitch/feed');
  const channels = feed.items.slice(0, 3);
  if (!channels.length) throw new Error('No followed live Twitch channels');
  const page = await context.newPage();
  await page.goto(`${origin}/?section=Twitch`);
  const hold = Number(process.env.THELXINOE_BENCHMARK_HOLD_SECONDS ?? 0);
  if (!Number.isFinite(hold) || hold < 0 || hold > 60)
    throw new Error('Invalid playback hold time');
  for (let repeat = 0; repeat < 2; repeat++) {
    for (const [index, channel] of channels.entries()) {
      const requests = new Map();
      const stages = {};
      const request = (r) => {
        const path = new URL(r.url()).pathname;
        if (
          path.endsWith('/playback') ||
          (r.method() === 'POST' && path === '/api/v1/playback')
        ) {
          requests.set(r, {
            start: performance.now(),
            name: r.method() === 'GET' ? 'info_ms' : 'prepare_ms',
          });
        }
      };
      const response = (r) => {
        const value = requests.get(r.request());
        if (value)
          stages[value.name] = Math.round(performance.now() - value.start);
      };
      page.on('request', request);
      page.on('response', response);
      const start = performance.now();
      await page
        .getByRole('button', {
          name: `Watch ${channel.display_name}`,
          exact: true,
        })
        .click();
      try {
        await page.waitForFunction(
          () => {
            const video = document.querySelector('video');
            return (
              video &&
              video.getVideoPlaybackQuality().totalVideoFrames > 0 &&
              video.currentTime > 0
            );
          },
          {},
          { timeout: 45000, polling: 20 },
        );
        results.push({
          channel: index,
          repeat,
          first_frame_ms: Math.round(performance.now() - start),
          ...stages,
        });
        if (hold && repeat === 0 && index === 0) {
          const startPosition = await page
            .locator('video')
            .evaluate((video) => video.currentTime);
          await page.waitForFunction(
            (position) => {
              const video = document.querySelector('video');
              return video && !video.error && video.currentTime >= position;
            },
            startPosition + hold,
            { timeout: (hold + 15) * 1000 },
          );
          results.at(-1).sustained_playback_seconds = hold;
        }
      } catch {
        const current = results.at(-1);
        if (current?.channel === index && current.repeat === repeat)
          current.failed = true;
        else results.push({ channel: index, repeat, failed: true, ...stages });
      } finally {
        page.off('request', request);
        page.off('response', response);
        writeFileSync(output, JSON.stringify({ results }, null, 2));
        // The live feed can rerender between locating the button and clicking it.
        const close = page.getByRole('button', {
          name: 'Close player',
          exact: true,
        });
        if (await close.count())
          await close.evaluate((button) => button.click());
        await page
          .locator('video')
          .waitFor({ state: 'detached', timeout: 5000 });
      }
      console.log(JSON.stringify(results.at(-1)));
    }
  }
  if (results.some((result) => result.failed))
    throw new Error('One or more playback attempts failed');
} finally {
  await api('/auth/logout', 'POST').catch(() => {});
  await browser.close();
}
