// Run against an explicitly selected idle development account. Videos must be
// in its watchlist; this updates their playback progress. Results omit URLs,
// credentials and grants. Use the same IDs and tool versions for comparisons.
import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';

const origin = process.env.THELXINOE_BENCHMARK_URL;
const ids = process.argv.slice(2);
if (!origin || !ids.length || ids.some((id) => !/^[\w-]{11}$/.test(id)))
  throw new Error('Set THELXINOE_BENCHMARK_URL and supply YouTube video IDs');
const output =
  process.env.THELXINOE_BENCHMARK_OUTPUT ??
  '.local/youtube-performance/browser.json';
mkdirSync(dirname(output), { recursive: true });
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
const results = [];
const api = async (path, method = 'GET', data) => {
  const response = await context.request.fetch(`${origin}/api/v1${path}`, {
    method,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
    timeout: 150000,
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
  const tools = await api('/admin/tools');
  const feed = await api('/online/youtube/feed?watchlist=true');
  const page = await context.newPage();
  // Disposable public-video fixtures need no Google account. Only expose the
  // provider UI here; authorization, extraction and all playback requests still
  // use the real server and the signed-in development account.
  if (process.env.THELXINOE_BENCHMARK_PUBLIC_FIXTURE === '1')
    await page.route('**/api/v1/online/youtube', (route) =>
      route.fulfill({
        json: { configured: true, account: { status: 'connected' }, sync: {} },
      }),
    );
  await page.goto(`${origin}/?section=YouTube`);
  const search = page.getByRole('searchbox');
  await search.waitFor();
  for (const id of ids) {
    const video = feed.items.find((item) => item.id === id);
    if (!video || video.download)
      throw new Error(`${id}: expected an undownloaded watchlist video`);
    for (const repeat of [0, 1]) {
      const requests = new Map();
      const row = {
        video: id,
        repeat,
        yt_dlp: tools.items.find((item) => item.id === 'yt-dlp')?.installed,
      };
      const mediaRequests = [];
      const request = (r) => {
        const path = new URL(r.url()).pathname;
        if (path.endsWith('/playback'))
          requests.set(r, {
            start: performance.now(),
            name: r.method() === 'GET' ? 'info_ms' : 'prepare_ms',
          });
      };
      const response = (r) => {
        const path = new URL(r.url()).pathname;
        if (path.includes('/hls/'))
          mediaRequests.push({
            file: path.split('/').at(-1),
            status: r.status(),
          });
        const value = requests.get(r.request());
        if (value)
          row[value.name] = Math.round(performance.now() - value.start);
      };
      page.on('request', request);
      page.on('response', response);
      const prepared = page
        .waitForResponse(
          (r) =>
            new URL(r.url()).pathname === '/api/v1/playback' &&
            r.request().method() === 'POST',
        )
        .catch(() => null);
      await search.fill(id);
      const start = performance.now();
      try {
        await search.press('Enter');
        const preparedResponse = await prepared;
        if (!preparedResponse || !preparedResponse.ok())
          throw new Error('Playback preparation failed');
        const session = await preparedResponse.json();
        row.mode = session.mode;
        row.quality = session.quality;
        await page.waitForFunction(
          () => {
            const v = document.querySelector('video');
            return v && v.getVideoPlaybackQuality().totalVideoFrames > 0;
          },
          {},
          { timeout: 45000, polling: 20 },
        );
        row.first_frame_ms = Math.round(performance.now() - start);
        const initial = await page
          .locator('video')
          .evaluate((v) => v.currentTime);
        await page.waitForFunction(
          (position) => {
            const v = document.querySelector('video');
            return v && !v.error && v.currentTime >= position + 10;
          },
          initial,
          { timeout: 25000 },
        );
        row.sustained_playback_seconds = 10;
        if (repeat === 0) {
          const seekStart = performance.now();
          const seekResponse = page.waitForResponse(
            (r) =>
              new URL(r.url()).pathname.endsWith('/seek') &&
              r.request().method() === 'POST',
          );
          await page.locator('video').hover();
          const slider = page.getByLabel('Playback position', { exact: true });
          await slider.fill('60');
          await slider.dispatchEvent('change');
          const seek = await seekResponse;
          expect(seek.status()).toBe(200);
          const data = await seek.json();
          await page.waitForFunction(
            (offset) => {
              const v = document.querySelector('video');
              return (
                v &&
                v.readyState >= 2 &&
                !v.seeking &&
                !v.paused &&
                v.currentTime + offset >= 60
              );
            },
            data.timeline_start,
            { timeout: 30000 },
          );
          row.seek_ms = Math.round(performance.now() - seekStart);
          await page.waitForFunction(
            (offset) => {
              const v = document.querySelector('video');
              return v && !v.error && v.currentTime + offset >= 70;
            },
            data.timeline_start,
            { timeout: 25000 },
          );
          row.sustained_seek_seconds = 10;
        } else {
          row.resumed_at = session.position;
          expect(session.position).toBeGreaterThanOrEqual(59);
        }
        row.decoded_frames = await page
          .locator('video')
          .evaluate((v) => v.getVideoPlaybackQuality().totalVideoFrames);
      } catch (error) {
        row.failed = true;
        row.media_requests = mediaRequests;
        row.player = await page
          .locator('video')
          .evaluate((v) => ({
            position: v.currentTime,
            paused: v.paused,
            ready_state: v.readyState,
            error_code: v.error?.code,
            buffered: Array.from({ length: v.buffered.length }, (_, i) => [
              v.buffered.start(i),
              v.buffered.end(i),
            ]),
          }))
          .catch(() => null);
        // Errors from provider responses can contain sensitive data; retain
        // only the stage timings, not raw exceptions or response bodies.
        console.error(`Playback attempt failed (${error.name})`);
      } finally {
        page.off('request', request);
        page.off('response', response);
        const close = page.getByRole('button', {
          name: 'Close player',
          exact: true,
        });
        if (await close.count())
          await close.evaluate((button) => button.click());
        await page.locator('video').waitFor({ state: 'detached' });
        results.push(row);
        writeFileSync(output, JSON.stringify({ results }, null, 2));
        console.log(JSON.stringify(row));
      }
    }
    const native = await api('/playback', 'POST', {
      media_id: `youtube:${id}`,
      position: 60,
      options: {
        quality: 'auto',
        capabilities: {
          containers: ['mp4', 'webm'],
          video: ['h264', 'vp9', 'av1'],
          audio: ['aac', 'opus'],
          hls: true,
          native_remote: true,
        },
      },
    });
    try {
      expect(native.mode).toBe('direct');
      for (const path of [native.url, native.external_audio].filter(Boolean)) {
        const range = await context.request.get(`${origin}${path}`, {
          headers: { Range: 'bytes=0-4095' },
        });
        expect(range.status()).toBe(206);
        expect((await range.body()).length).toBe(4096);
      }
      results.push({ video: id, native_direct_ranges: true });
      writeFileSync(output, JSON.stringify({ results }, null, 2));
    } finally {
      await api(`/playback/${native.id}`, 'DELETE');
    }
  }
  if (results.some((row) => row.failed))
    throw new Error('One or more playback attempts failed; see the results');
} finally {
  await api('/auth/logout', 'POST').catch(() => {});
  await browser.close();
}
