import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmdirSync,
  unlinkSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// Exercise the real player with decoded media and a controlled server boundary.
// This suite starts its own Vite server and never touches an account or deployment.
const title = 'A journey through the mountains';
let mp4: Buffer, segment: Buffer;
test.beforeAll(() => {
  const directory = mkdtempSync(join(tmpdir(), 'thelxinoe-player-'));
  const file = join(directory, 'fixture.mp4');
  try {
    execFileSync(
      'ffmpeg',
      [
        '-hide_banner',
        '-loglevel',
        'error',
        '-f',
        'lavfi',
        '-i',
        'color=c=0x243841:size=960x540:rate=12',
        '-t',
        '60',
        '-an',
        '-c:v',
        'libx264',
        '-preset',
        'ultrafast',
        '-g',
        '24',
        '-pix_fmt',
        'yuv420p',
        '-movflags',
        '+faststart',
        '-f',
        'mp4',
        file,
      ],
      { windowsHide: true, maxBuffer: 8 * 1024 * 1024 },
    );
    mp4 = readFileSync(file);
  } finally {
    try {
      unlinkSync(file);
    } finally {
      rmdirSync(directory);
    }
  }
  segment = execFileSync(
    'ffmpeg',
    [
      '-hide_banner',
      '-loglevel',
      'error',
      '-i',
      'pipe:0',
      '-c',
      'copy',
      '-f',
      'mpegts',
      'pipe:1',
    ],
    { input: mp4, windowsHide: true, maxBuffer: 8 * 1024 * 1024 },
  );
  mkdirSync('.local/player-ui', { recursive: true });
});
test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: 'ignoreErrors' });
});

function gate(hold: boolean) {
  let release!: () => void;
  const promise = new Promise<void>((resolve) => {
    release = resolve;
  });
  if (!hold) release();
  return { promise, release };
}
async function fixture(
  page: Page,
  options: {
    hold?: boolean;
    mode?: string;
    live?: boolean;
    fail?: boolean;
    app?: boolean;
  } = {},
) {
  const metadata = gate(!!options.hold),
    preparation = gate(!!options.hold),
    media = gate(!!options.hold);
  const state = {
    metadata,
    preparation,
    media,
    starts: [] as { position: number; options: { quality: string } }[],
    seeks: [] as number[],
    progress: [] as { state: string; position: number }[],
    errors: [] as string[],
    savedVolume: 0.31,
    fail: options.fail ?? false,
  };
  page.on('pageerror', (error) => state.errors.push(error.message));
  const videos = Array.from({ length: 80 }, (_, i) => ({
    videoId: `video${i}`,
    channelId: 'channel',
    channelName: 'Fixture channel',
    title: `Video ${i}`,
    thumbnailUrl:
      'data:image/svg+xml,%3Csvg xmlns="http://www.w3.org/2000/svg"/%3E',
    publishedAt: '2026-09-21T00:00:00Z',
    durationSeconds: 60,
    isLive: false,
    isUpcoming: false,
    isLiveReplay: false,
    broadcastState: 'none',
    positionSeconds: 0,
    watchedPercentage: 0,
    isWatched: false,
  }));
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (options.app) {
      if (path === '/api/v1/health')
        return route.fulfill({ json: { api_version: 1 } });
      if (path === '/api/v1/setup')
        return route.fulfill({ json: { setup_required: false } });
      if (path === '/api/v1/me/notifications')
        return route.fulfill({ json: { items: [] } });
      if (path === '/api/v1/auth/me')
        return route.fulfill({
          json: {
            user: {
              id: 'fixture',
              username: 'Viewer',
              role: 'user',
              timezone: 'UTC',
            },
          },
        });
      if (path === '/api/v1/auth/event-ticket')
        return route.fulfill({
          status: 503,
          json: { error: { message: 'Events disabled in fixture' } },
        });
      if (path === '/api/v1/online/twitch' || path === '/api/v1/online/youtube')
        return route.fulfill({
          json: {
            configured: true,
            account: {
              status: 'connected',
              updated_at: 0,
            },
            sync: {},
          },
        });
      if (path === '/api/v1/online/kick')
        return route.fulfill({
          json: {
            configured: true,
            items: videos.map((_, i) => ({
              slug: `channel${i}`,
              title,
              category: 'Travel',
              live: true,
              viewers: 80 - i,
              updated_at: 0,
              error: null,
            })),
          },
        });
      if (path === '/api/v1/online/youtube/watchlists')
        return route.fulfill({
          json: [
            {
              id: 1,
              name: 'Watch Later',
              isDefault: true,
              autoDownload: false,
              autoRemoveWatched: false,
              sortMode: 'manual',
              sortDirection: 'desc',
              createdAt: '',
              updatedAt: '',
              items: videos.map((video, manualPosition) => ({
                video,
                manualPosition,
                addedAt: '',
              })),
            },
          ],
        });
      if (path === '/api/v1/online/youtube/browse')
        return route.fulfill({
          json: {
            items: videos,
            page: 0,
            pageSize: 80,
            hasMore: false,
            channels: [],
            counts: {
              all: 80,
              unwatched: 80,
              inProgress: 0,
              watched: 0,
              shorts: 0,
              live: 0,
              liveReplays: 0,
              upcoming: 0,
              subscribedChannelCount: 1,
            },
          },
        });
      if (path === '/api/v1/online/twitch/feed')
        return route.fulfill({
          json: {
            items: Array.from({ length: 30 }, (_, i) => ({
              id: String(i),
              login: `channel${i}`,
              display_name: `Channel ${i}`,
              title,
              category: 'Travel',
              viewers: 30 - i,
              started_at: '2026-09-21T00:00:00Z',
            })),
          },
        });
    }
    if (path.endsWith('/me/appearance')) {
      if (route.request().method() === 'PATCH')
        state.savedVolume =
          route.request().postDataJSON().audio_volume ?? state.savedVolume;
      return route.fulfill({
        json: { theme: 'dark', audio_volume: state.savedVolume },
      });
    }
    if (path.startsWith('/api/v1/catalog/') && path.endsWith('/playback')) {
      await metadata.promise;
      return route.fulfill({
        json: {
          sources: [],
          preferences: { quality: 'auto', subtitles: false },
          progress: [],
          watched: false,
        },
      });
    }
    if (path === '/api/v1/playback') {
      const input = route.request().postDataJSON();
      state.starts.push(input);
      await preparation.promise;
      if (state.fail)
        return route.fulfill({
          status: 503,
          json: {
            error: {
              code: 'fixture',
              message: 'The stream is temporarily unavailable.',
            },
          },
        });
      return route.fulfill({
        json: {
          id: 'fixture',
          mode: options.mode ?? 'direct',
          live: options.live ?? false,
          url:
            options.mode === 'remux'
              ? '/fixture/index.m3u8'
              : '/fixture/video.mp4',
          position: input.position ?? 0,
          duration: 60,
          timeline_start: 0,
          video: true,
          tracks: [],
          subtitles: [],
          selected_subtitle: 'off',
          options: input.options,
          probe: {},
          replay_gain: 'off',
        },
      });
    }
    if (path.endsWith('/seek')) {
      const { position } = route.request().postDataJSON();
      state.seeks.push(position);
      return route.fulfill({
        json: {
          url: `/fixture/index.m3u8?at=${position}`,
          timeline_start: position,
          position,
        },
      });
    }
    if (path.endsWith('/progress'))
      state.progress.push(route.request().postDataJSON());
    return route.fulfill({ json: {} });
  });
  await page.route(
    /\/fixture\/(?:video\.mp4|index\.m3u8|segment\.ts)(?:\?.*)?$/,
    async (route) => {
      await media.promise;
      const url = new URL(route.request().url());
      const range = route.request().headers().range;
      if (url.pathname.endsWith('.mp4') && range) {
        const [, from, to] = /^bytes=(\d+)-(\d*)$/.exec(range)!;
        const start = Number(from),
          end = to ? Math.min(Number(to), mp4.length - 1) : mp4.length - 1;
        return route.fulfill({
          status: 206,
          contentType: 'video/mp4',
          headers: {
            'Accept-Ranges': 'bytes',
            'Content-Range': `bytes ${start}-${end}/${mp4.length}`,
          },
          body: mp4.subarray(start, end + 1),
        });
      }
      if (url.pathname.endsWith('.m3u8'))
        return route.fulfill({
          contentType: 'application/vnd.apple.mpegurl',
          body: '#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:60\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:60,\nsegment.ts\n#EXT-X-ENDLIST\n',
        });
      return route.fulfill({
        contentType: url.pathname.endsWith('.ts') ? 'video/mp2t' : 'video/mp4',
        body: url.pathname.endsWith('.ts') ? segment : mp4,
      });
    },
  );
  await page.route('**/player-test*', (route) =>
    route.fulfill({
      contentType: 'text/html',
      body: `<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1"></head><body style="margin:0;padding:${options.app ? 0 : 16}px"><main id="fixture-root"></main>
      <script type="module">
        import { mount, unmount } from '/node_modules/.vite/deps/svelte.js';
        import Player from '/src/lib/Player.svelte';
        import App from '/src/App.svelte';
        import { loadAppearance } from '/src/lib/appearance.ts';
        import '/src/app.css';
        await loadAppearance('fixture');
        const app = ${
          options.app
            ? "mount(App, { target: document.querySelector('#fixture-root') })"
            : `mount(Player, { target: document.querySelector('#fixture-root'), props: {
          choice: { id: 'fixture', title: ${JSON.stringify(title)} }, closed: () => unmount(app, { outro: true })
        }})`
        };
      </script></body></html>`,
    }),
  );
  await page.goto(`/player-test${options.app ? '?section=Twitch' : ''}`);
  return state;
}
async function decoded(page: Page) {
  await expect
    .poll(() =>
      page
        .locator('video')
        .evaluate(
          (video: HTMLVideoElement) =>
            video.getVideoPlaybackQuality().totalVideoFrames,
        ),
    )
    .toBeGreaterThan(0);
  await expect(page.getByRole('status')).toHaveCount(0);
}
async function insidePlayer(page: Page) {
  for (const locator of [
    page.getByRole('heading', { name: title }),
    page.locator('.control-row'),
    page.getByRole('button', { name: 'Quality', exact: true }),
    page.getByRole('button', { name: 'Audio', exact: true }),
    page.getByRole('button', { name: 'Subtitles', exact: true }),
    page.getByRole('button', { name: 'Close player' }),
  ]) {
    const { player, box } = await locator.evaluate((element) => ({
      player: element.closest('.player')!.getBoundingClientRect().toJSON(),
      box: element.getBoundingClientRect().toJSON(),
    }));
    expect(box.x).toBeGreaterThanOrEqual(player.x);
    expect(box.y).toBeGreaterThanOrEqual(player.y);
    expect(box.x + box.width).toBeLessThanOrEqual(player.x + player.width + 1);
    expect(box.y + box.height).toBeLessThanOrEqual(
      player.y + player.height + 1,
    );
  }
}

test('player appears before preparation and stays loading until video frames arrive', async ({
  page,
}) => {
  const state = await fixture(page, { hold: true });
  const player = page.getByRole('region', { name: 'Media player' });
  await expect(player).toBeVisible();
  await expect(page.locator('video')).toBeVisible();
  await expect(page.getByRole('heading', { name: title })).toBeVisible();
  await expect(page.getByRole('status')).toContainText('Preparing playback');
  const before = (await player.boundingBox())!;
  await insidePlayer(page);
  await page.screenshot({ path: '.local/player-ui/loading.png' });
  state.metadata.release();
  await expect.poll(() => state.starts.length).toBe(1);
  await expect(page.getByRole('status')).toBeVisible();
  state.preparation.release();
  await expect(page.getByRole('status')).toContainText('Buffering');
  expect(
    await page.locator('video').evaluate((v: HTMLVideoElement) => v.readyState),
  ).toBe(0);
  state.media.release();
  await decoded(page);
  expect((await player.boundingBox())!.height).toBeCloseTo(before.height, 0);
  await page.locator('video').dispatchEvent('waiting');
  await expect(page.getByRole('status')).toContainText('Buffering');
  await page.locator('video').dispatchEvent('playing');
  await expect(page.getByRole('status')).toHaveCount(0);
  expect(state.errors).toEqual([]);
});

test('controls hide, recover with keyboard, retain volume, seek, and stay inside fullscreen and mobile video', async ({
  page,
}) => {
  const state = await fixture(page);
  await decoded(page);
  await page.mouse.move(1, 1);
  await expect(page.locator('.player-controls')).toHaveCSS('opacity', '0', {
    timeout: 5000,
  });
  await page.keyboard.press('Tab');
  await expect(
    page.getByRole('button', { name: 'Close player' }),
  ).toBeFocused();
  await expect(page.locator('.player-controls')).toHaveCSS('opacity', '1');
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  const volume = page.getByRole('slider', { name: 'Playback volume' });
  await volume.fill('0.23');
  await expect.poll(() => state.savedVolume).toBe(0.23);
  await page.getByRole('button', { name: 'Mute', exact: true }).click();
  expect(
    await page.locator('video').evaluate((v: HTMLVideoElement) => v.muted),
  ).toBe(true);
  await page.getByRole('button', { name: 'Unmute', exact: true }).click();
  await expect(volume).toHaveValue('0.23');
  await page.getByRole('slider', { name: 'Playback position' }).fill('8');
  await expect
    .poll(() =>
      page.locator('video').evaluate((v: HTMLVideoElement) => v.currentTime),
    )
    .toBeCloseTo(8, 0);
  expect(state.seeks).toEqual([]);
  await page.getByRole('button', { name: 'Fullscreen', exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        document.fullscreenElement?.classList.contains('player'),
      ),
    )
    .toBe(true);
  await insidePlayer(page);
  await page.screenshot({ path: '.local/player-ui/fullscreen.png' });
  await page.getByRole('button', { name: 'Exit fullscreen' }).click();
  await page.getByRole('button', { name: 'Quality', exact: true }).click();
  await expect(page.getByRole('menu', { name: 'Quality' })).toBeVisible();
  await page
    .getByRole('menuitemradio', { name: 'Original', exact: true })
    .click();
  await expect.poll(() => state.starts.length).toBe(2);
  expect(state.starts[1].options.quality).toBe('original');
  expect(state.starts[1].position).toBeGreaterThanOrEqual(8);
  await decoded(page);
  await expect(volume).toHaveValue('0.23');
  await page.getByRole('button', { name: 'Quality', exact: true }).click();
  await expect(
    page.getByRole('menuitemradio', { name: 'Original', exact: true }),
  ).toHaveAttribute('aria-checked', 'true');
  await page.screenshot({ path: '.local/player-ui/settings.png' });
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu')).toHaveCount(0);
  await page.getByRole('button', { name: 'Audio', exact: true }).click();
  await expect(page.getByRole('menu', { name: 'Audio' })).toBeVisible();
  await expect(
    page.getByRole('menuitemradio', { name: 'Default', exact: true }),
  ).toHaveAttribute('aria-checked', 'true');
  await page.getByRole('button', { name: 'Subtitles', exact: true }).click();
  await expect(page.getByRole('menu', { name: 'Audio' })).toHaveCount(0);
  await expect(
    page.getByRole('menuitemradio', { name: 'Off', exact: true }),
  ).toHaveAttribute('aria-checked', 'true');
  await page.keyboard.press('Escape');
  await expect(
    page.getByRole('group', { name: 'Playback settings' }),
  ).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await insidePlayer(page);
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  await page.screenshot({ path: '.local/player-ui/mobile.png' });
  expect(state.errors).toEqual([]);
});

test('transcoded seeking uses the server timeline and live playback omits seeking', async ({
  page,
}) => {
  const state = await fixture(page, { mode: 'remux' });
  await decoded(page);
  await page.getByRole('slider', { name: 'Playback position' }).fill('42');
  await expect.poll(() => state.seeks).toEqual([42]);
  await decoded(page);
  await expect
    .poll(async () =>
      Number(
        await page
          .getByRole('slider', { name: 'Playback position' })
          .inputValue(),
      ),
    )
    .toBeGreaterThanOrEqual(42);
  await page.getByRole('button', { name: 'Close player' }).click();
  await page.unrouteAll({ behavior: 'ignoreErrors' });
  const live = await fixture(page, { live: true, mode: 'remux' });
  await decoded(page);
  await expect(
    page.getByRole('slider', { name: 'Playback position' }),
  ).toHaveCount(0);
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  await page.getByRole('button', { name: 'Play', exact: true }).click();
  await expect
    .poll(() =>
      page.locator('video').evaluate((v: HTMLVideoElement) => v.paused),
    )
    .toBe(false);
  expect(live.seeks).toEqual([]);
  expect(state.errors).toEqual([]);
  expect(live.errors).toEqual([]);
});

test('the online player shares the page scrollbar and closes by sliding the feed upward', async ({
  page,
}) => {
  const state = await fixture(page, { app: true });
  await page
    .getByRole('button', { name: 'Watch Channel 0', exact: true })
    .waitFor();
  const feed = page.locator('.provider-surface');
  const closedTop = (await feed.boundingBox())!.y;
  // Pause the real Svelte animations at creation so intermediate geometry is
  // deterministic, including on slower CI runners.
  await page.evaluate(() => {
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (keyframes, options) {
      const animation = animate.call(this, keyframes, options);
      if (
        this.classList.contains('player') &&
        Number(animation.effect?.getTiming().duration) > 0
      )
        animation.pause();
      return animation;
    };
  });
  await page
    .getByRole('button', { name: 'Watch Channel 0', exact: true })
    .click();
  const player = page.getByRole('region', { name: 'Media player' });
  async function halfReveal() {
    await expect
      .poll(() =>
        player.evaluate((element) =>
          element
            .getAnimations()
            .some(
              (animation) => Number(animation.effect?.getTiming().duration) > 0,
            ),
        ),
      )
      .toBe(true);
    await player.evaluate((element) => {
      const animation = element.getAnimations()[0];
      if (!animation) throw new Error('Missing player slide animation');
      animation.currentTime =
        Number(animation.effect!.getTiming().duration) / 2;
    });
    return (await feed.boundingBox())!.y;
  }
  const openingTop = await halfReveal();
  expect(openingTop).toBeGreaterThan(closedTop);
  await page.screenshot({ path: '.local/player-ui/opening.png' });
  await player.evaluate((element) => element.getAnimations()[0].finish());
  await decoded(page);
  await expect(player).toHaveCSS('transform', 'none');
  const openTop = (await feed.boundingBox())!.y;
  expect(openingTop).toBeLessThan(openTop);
  const workspace = page.locator('.workspace-scroll');
  const initial = (await player.boundingBox())!;
  await workspace.evaluate((element) => {
    element.scrollTop = 300;
  });
  await expect
    .poll(async () => (await player.boundingBox())!.y)
    .toBeCloseTo(initial.y - 300, 0);
  expect(
    await page
      .locator('[data-feed-content]')
      .evaluate((element) => element.scrollTop),
  ).toBe(0);
  await workspace.evaluate((element) => {
    element.scrollTop = 0;
  });
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  const closingTop = await halfReveal();
  expect(closingTop).toBeGreaterThan(closedTop);
  expect(closingTop).toBeLessThan(openTop);
  await page.screenshot({ path: '.local/player-ui/closing.png' });
  await player.evaluate((element) => element.getAnimations()[0].finish());
  await expect(player).toHaveCount(0);
  await expect
    .poll(() => state.progress.some((event) => event.state === 'stopped'))
    .toBe(true);
  await expect(
    page.getByRole('button', { name: 'Watch Channel 0', exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: '.local/player-ui/feed.png' });
  expect(state.errors).toEqual([]);
});

test('YouTube and Kick use page scrolling while the watchlist stays within the viewport', async ({
  page,
}) => {
  const state = await fixture(page, { app: true });
  await page.getByRole('button', { name: 'YouTube', exact: true }).click();
  await expect(page.locator('[data-feed-content] [data-video-id]')).toHaveCount(
    80,
  );
  await page
    .getByRole('button', { name: 'Open watchlists', exact: true })
    .click();
  const workspace = page.locator('.workspace-scroll');
  const sidebar = page.locator('[data-youtube-watchlist-frame]');
  await expect(sidebar).toHaveAttribute('aria-hidden', 'false');
  const viewport = (await workspace.boundingBox())!;
  const inset = await workspace.evaluate((element) =>
    Number.parseFloat(getComputedStyle(element).paddingTop),
  );
  expect((await sidebar.boundingBox())!.height).toBeLessThanOrEqual(
    viewport.height,
  );
  const card = page.locator('[data-feed-content] [data-video-id]').first();
  const before = (await card.boundingBox())!.y;
  await workspace.evaluate((element) => {
    element.scrollTop = 350;
  });
  await expect
    .poll(async () => (await card.boundingBox())!.y)
    .toBeCloseTo(before - 350, 0);
  await expect
    .poll(async () => (await sidebar.boundingBox())!.y)
    .toBeCloseTo(viewport.y + inset, 0);
  await expect(
    page.locator('[data-card-group-header][data-stuck]'),
  ).toHaveCount(1);
  await page.screenshot({ path: '.local/player-ui/youtube-scroll.png' });
  await page.getByRole('button', { name: 'Kick', exact: true }).click();
  await expect
    .poll(() => workspace.evaluate((element) => element.scrollTop))
    .toBe(0);
  await expect(
    page.locator('[data-feed-content] [data-card-grid] > *'),
  ).toHaveCount(80);
  const feed = page.locator('[data-feed-content]');
  const initial = (await feed.boundingBox())!.y;
  await workspace.evaluate((element) => {
    element.scrollTop = 350;
  });
  await expect
    .poll(async () => (await feed.boundingBox())!.y)
    .toBeCloseTo(initial - 350, 0);
  expect(state.errors).toEqual([]);
});

test('failed streams can retry and closing during preparation stops a late session', async ({
  page,
}) => {
  const state = await fixture(page, { fail: true });
  await expect(page.getByRole('alert')).toContainText(
    'The stream is temporarily unavailable',
  );
  await expect(page.getByRole('status')).toHaveCount(0);
  state.fail = false;
  await page.getByRole('button', { name: 'Retry playback' }).click();
  await decoded(page);
  await page.getByRole('button', { name: 'Close player' }).click();
  await expect(page.locator('video')).toHaveCount(0);
  await page.unrouteAll({ behavior: 'ignoreErrors' });
  const pending = await fixture(page, { hold: true });
  pending.metadata.release();
  await expect.poll(() => pending.starts.length).toBe(1);
  await page.getByRole('button', { name: 'Close player' }).click();
  await expect(page.getByRole('region', { name: 'Media player' })).toHaveCount(
    0,
  );
  pending.preparation.release();
  await expect
    .poll(() => pending.progress.some((p) => p.state === 'stopped'))
    .toBe(true);
  expect(pending.errors).toEqual([]);
});
