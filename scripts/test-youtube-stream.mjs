// Live-provider streaming check on the isolated HTTPS deployment.
import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const origin = 'https://localhost:22443';
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
async function api(path, method = 'GET', data) {
  const response = await context.request.fetch(`${origin}/api/v1${path}`, {
    method,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
    timeout: 150000,
  });
  if (!response.ok()) throw Error(`${path}: HTTP ${response.status()}`);
  return response.json();
}
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const feed = await api('/online/youtube/feed?hide_shorts=true');
  const video = feed.items.find(
    (v) =>
      v.privacy === 'public' &&
      v.broadcast === 'none' &&
      v.duration > 30 &&
      !v.download,
  );
  expect(video).toBeTruthy();
  const page = await context.newPage();
  await page.goto(`${origin}/?section=YouTube`);
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
    { timeout: 60000 },
  );
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  const frames = await page
    .locator('video')
    .evaluate((v) => v.getVideoPlaybackQuality().totalVideoFrames);
  const seek = page.getByLabel('Playback position', { exact: true });
  const seekResponse = page.waitForResponse(
    (r) => r.url().includes('/seek') && r.request().method() === 'POST',
  );
  await seek.fill('10');
  await seek.dispatchEvent('change');
  expect((await seekResponse).status()).toBe(200);
  await expect(
    page.getByRole('button', { name: 'Pause', exact: true }),
  ).toBeEnabled({ timeout: 30000 });
  await expect
    .poll(async () => Number(await seek.inputValue()), { timeout: 30000 })
    .toBeGreaterThanOrEqual(10);
  await page.waitForFunction(
    () =>
      document.querySelector('video')?.getVideoPlaybackQuality()
        .totalVideoFrames > 0,
    {},
    { timeout: 30000 },
  );
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await expect
    .poll(
      async () =>
        (await api(`/catalog/youtube:${video.id}/playback`)).progress[0]
          .position,
    )
    .toBeGreaterThanOrEqual(9.8);
  expect(
    (await api(`/online/youtube/videos/${video.id}/download`)).download,
  ).toBeNull();
  const result = {
    streamed_without_download: true,
    decoded_frames: frames,
    seek: true,
    resume: true,
    https: true,
  };
  writeFileSync(
    '.local/youtube-stream-result.json',
    JSON.stringify(result, null, 2),
  );
  console.log(JSON.stringify(result));
} finally {
  await api('/auth/logout', 'POST').catch(() => {});
  await browser.close();
}
