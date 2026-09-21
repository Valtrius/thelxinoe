import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
const origin = 'https://localhost:22443';
async function api(path, method = 'GET', data) {
  const r = await context.request.fetch(`${origin}/api/v1${path}`, {
    method,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
  });
  if (!r.ok()) throw Error(`${path}: HTTP ${r.status()}`);
  return r.json();
}
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const feed = await api('/online/twitch/feed');
  expect(feed.items.length).toBeGreaterThan(0);
  const stream = feed.items[0];
  const page = await context.newPage();
  await page.goto(`${origin}/?section=Twitch`);
  await page
    .getByRole('button', { name: `Play ${stream.display_name}`, exact: true })
    .click();
  await page.waitForFunction(
    () => {
      const v = document.querySelector('video');
      return (
        v &&
        v.currentTime > 0.5 &&
        v.getVideoPlaybackQuality().totalVideoFrames > 0
      );
    },
    {},
    { timeout: 60000 },
  );
  await expect(
    page.getByLabel('Playback position', { exact: true }),
  ).toHaveCount(0);
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  const frames = await page
    .locator('video')
    .evaluate((v) => v.getVideoPlaybackQuality().totalVideoFrames);
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await expect
    .poll(
      async () =>
        (await api('/me/history?platform=twitch&range=all')).items.length,
    )
    .toBeGreaterThan(0);
  const result = {
    https: true,
    public_streamlink: true,
    decoded_frames: frames,
    live_seek_hidden: true,
    history: true,
  };
  writeFileSync(
    '.local/twitch-playback-result.json',
    JSON.stringify(result, null, 2),
  );
  console.log(JSON.stringify(result));
} finally {
  await api('/auth/logout', 'POST').catch(() => {});
  await browser.close();
}
