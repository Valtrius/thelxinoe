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
let cleanup = async () => {};
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const feed = await api('/online/kick');
  const slug = process.argv[2] || 'starladder';
  if (!/^[a-z0-9_-]+$/.test(slug)) throw Error('Invalid fixture channel');
  const existed = feed.items.some((c) => c.slug === slug);
  await api('/online/kick/channels', 'POST', { channel: slug });
  let channel;
  await expect
    .poll(
      async () => {
        channel = (await api('/online/kick')).items.find(
          (c) => c.slug === slug,
        );
        return channel?.live;
      },
      { timeout: 60000 },
    )
    .toBe(true);
  cleanup = async () => {
    if (!existed) await api(`/online/kick/channels/${slug}`, 'DELETE');
    if (!feed.connected) await api('/online/kick', 'DELETE');
  };
  const page = await context.newPage();
  await page.goto(origin);
  await page.getByRole('button', { name: 'Kick', exact: true }).click();
  await page.getByRole('button', { name: `Play ${slug}`, exact: true }).click();
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
    .poll(async () => (await api('/me/history?domain=kick')).stats.plays)
    .toBeGreaterThan(0);
  const result = {
    https: true,
    public_streamlink: true,
    decoded_frames: frames,
    live_seek_hidden: true,
    history: true,
  };
  writeFileSync(
    '.local/kick-playback-result.json',
    JSON.stringify(result, null, 2),
  );
  console.log(JSON.stringify(result));
} finally {
  await cleanup().catch(() => {});
  await api('/auth/logout', 'POST').catch(() => {});
  await browser.close();
}
