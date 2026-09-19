// Public-media smoke test for the isolated HTTPS deployment. Does not change
// application credentials or disconnect the live provider account.
import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const origin = 'https://localhost:22443';
const videoId = 'jNQXAC9IVRw';
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
async function api(path, method = 'GET', data) {
  const response = await context.request.fetch(`${origin}/api/v1${path}`, {
    method,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
  });
  if (!response.ok()) throw new Error(`${path}: HTTP ${response.status()}`);
  return response.json();
}
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const state = await api(`/online/youtube/videos/${videoId}/download`);
  expect(state.download?.state).toBe('ready');
  const feed = await api('/online/youtube/feed?watchlist=true');
  const video = feed.items.find((v) => v.id === videoId);
  expect(video).toBeTruthy();
  const page = await context.newPage();
  await page.goto(`${origin}/?section=YouTube`);
  await page.getByRole('button', { name: 'Watchlist', exact: true }).click();
  await page.getByLabel('Hide Shorts').uncheck();
  await page
    .getByRole('button', { name: `Play ${video.title}`, exact: true })
    .click();
  await page.waitForFunction(
    () => {
      const v = document.querySelector('video');
      return (
        v &&
        v.currentTime > 0.4 &&
        v.getVideoPlaybackQuality().totalVideoFrames > 0
      );
    },
    {},
    { timeout: 30000 },
  );
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  const frames = await page
    .locator('video')
    .evaluate((v) => v.getVideoPlaybackQuality().totalVideoFrames);
  const position = page.getByLabel('Playback position', { exact: true });
  await position.fill('8');
  await position.dispatchEvent('change');
  await expect
    .poll(() => page.locator('video').evaluate((v) => v.currentTime))
    .toBeGreaterThan(7);
  await page.screenshot({ path: '.local/youtube-playback.png' });
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  const info = await api(`/catalog/youtube:${videoId}/playback`);
  expect(info.progress[0].position).toBeGreaterThan(7);
  const result = {
    download: true,
    decoded_frames: frames,
    seek: true,
    resume: true,
    https: true,
  };
  writeFileSync(
    '.local/youtube-playback-result.json',
    JSON.stringify(result, null, 2),
  );
  console.log(JSON.stringify(result));
} finally {
  await api('/auth/logout', 'POST').catch(() => {});
  await browser.close();
}
