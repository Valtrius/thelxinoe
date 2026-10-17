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
import { freezeClock } from './helpers/clock';

// Exercise the real player with decoded media and a controlled server boundary.
// This suite starts its own Vite server and never touches an account or deployment.
const title = 'A journey through the mountains';
let mp4: Buffer, segment: Buffer, shortSegment: Buffer;
// Separate video and audio files shaped like YouTube's: an initialization
// segment, a `sidx` fragment index, then keyframe-aligned fragments.
let original: Record<'video' | 'audio', Buffer>;
function fragmentIndex(file: Buffer) {
  for (let offset = 0; offset < file.length;) {
    const size = file.readUInt32BE(offset);
    if (file.toString('latin1', offset + 4, offset + 8) !== 'sidx') {
      offset += size;
      continue;
    }
    // Version 1 widens the earliest presentation time and first offset.
    const wide = file[offset + 8] !== 0;
    const timescale = file.readUInt32BE(offset + 16);
    const first = wide
      ? Number(file.readBigUInt64BE(offset + 28))
      : file.readUInt32BE(offset + 24);
    const references = offset + (wide ? 36 : 28);
    const count = file.readUInt16BE(references + 2);
    let start = offset + size + first;
    const fragments = Array.from({ length: count }, (_, index) => {
      const at = references + 4 + index * 12;
      const fragment = {
        offset: start,
        size: file.readUInt32BE(at) & 0x7fffffff,
        duration: file.readUInt32BE(at + 4) / timescale,
      };
      start += fragment.size;
      return fragment;
    });
    return { init: file.subarray(0, offset), fragments };
  }
  throw new Error('Missing fragment index');
}
/** The server's byte-range playlist for one track of an indexed session. */
function indexedPlaylist(track: 'video' | 'audio') {
  const { fragments } = fragmentIndex(original[track]);
  return [
    '#EXTM3U',
    '#EXT-X-VERSION:7',
    '#EXT-X-PLAYLIST-TYPE:VOD',
    `#EXT-X-TARGETDURATION:${Math.ceil(Math.max(...fragments.map((f) => f.duration)))}`,
    '#EXT-X-MEDIA-SEQUENCE:0',
    '#EXT-X-INDEPENDENT-SEGMENTS',
    `#EXT-X-MAP:URI="../../remote/${track}-init?grant=fixture"`,
    ...fragments.flatMap((f) => [
      `#EXTINF:${f.duration.toFixed(6)},`,
      `#EXT-X-BYTERANGE:${f.size}@${f.offset}`,
      `../../remote/${track}?grant=fixture`,
    ]),
    '#EXT-X-ENDLIST',
    '',
  ].join('\n');
}
test.beforeAll(() => {
  const directory = mkdtempSync(join(tmpdir(), 'thelxinoe-player-'));
  const file = join(directory, 'fixture.mp4');
  const fragmented = [
    '-movflags',
    '+frag_keyframe+empty_moov+default_base_moof+global_sidx',
  ];
  for (const [name, args] of [
    [
      'video.mp4',
      [
        '-f',
        'lavfi',
        '-i',
        'testsrc2=size=960x540:rate=12',
        '-t',
        '60',
        '-c:v',
        'libx264',
        '-preset',
        'ultrafast',
        '-g',
        '24',
        '-pix_fmt',
        'yuv420p',
        '-an',
        ...fragmented,
      ],
    ],
    [
      'audio.m4a',
      [
        '-f',
        'lavfi',
        '-i',
        'sine=frequency=440:sample_rate=48000',
        '-t',
        '60',
        '-c:a',
        'aac',
        '-vn',
        '-frag_duration',
        '4000000',
        '-movflags',
        '+empty_moov+default_base_moof+global_sidx',
      ],
    ],
  ] as const) {
    const output = join(directory, name);
    execFileSync(
      'ffmpeg',
      ['-hide_banner', '-loglevel', 'error', ...args, '-f', 'mp4', output],
      { windowsHide: true },
    );
    original = {
      ...original,
      [name.startsWith('video') ? 'video' : 'audio']: readFileSync(output),
    };
    unlinkSync(output);
  }
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
  shortSegment = execFileSync(
    'ffmpeg',
    [
      '-v',
      'error',
      '-i',
      'pipe:0',
      '-t',
      '4',
      '-c',
      'copy',
      '-f',
      'mpegts',
      'pipe:1',
    ],
    { input: mp4, windowsHide: true },
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
    playerHeight?: number | null;
    subtitles?: boolean;
    qualities?: { value: string; label: string }[];
    autoQuality?: string;
    local?: boolean;
    stalled?: boolean;
    tools?: 'metadata' | 'playback';
    music?: boolean;
    native?: boolean;
    nativeTools?: boolean;
    nativeRestore?: boolean;
    indexed?: boolean;
  } = {},
) {
  const metadata = gate(!!options.hold),
    preparation = gate(!!options.hold),
    media = gate(!!options.hold);
  const state = {
    metadata,
    preparation,
    media,
    starts: [] as {
      media_id: string;
      position: number;
      options: { quality: string };
    }[],
    seeks: [] as number[],
    ranges: [] as string[],
    // Byte ranges of the indexed media that wait for a release.
    held: {
      test: (() => false) as (track: string, offset: number) => boolean,
      ...gate(true),
    },
    progress: [] as {
      state: string;
      position: number;
      active_seconds: number;
    }[],
    errors: [] as string[],
    savedVolume: 0.31,
    savedHeight: options.playerHeight ?? null,
    fail: options.fail ?? false,
    toolsPreparing: !!options.tools,
    toolsFailed: false,
    resolvePreparing: false,
    autoQuality: options.autoQuality ?? '1080p',
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
    if (path === '/api/v1/online/youtube/resolve' && state.resolvePreparing)
      return route.fulfill({
        status: 503,
        json: {
          error: {
            code: 'tools_preparing',
            message: 'Waiting for playback tools…',
          },
        },
      });
    if (options.app) {
      if (path === '/api/v1/online/providers')
        return route.fulfill({
          json: { youtube: true, twitch: true, kick: true },
        });
      if (path === '/api/v1/health')
        return route.fulfill({ json: { api_version: 1 } });
      if (path === '/api/v1/setup')
        return route.fulfill({ json: { setup_required: false } });
      if (path === '/api/v1/me/attention')
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
      if (route.request().method() === 'PATCH') {
        const change = route.request().postDataJSON();
        state.savedVolume = change.audio_volume ?? state.savedVolume;
        if ('player_height' in change) state.savedHeight = change.player_height;
      }
      return route.fulfill({
        json: {
          theme: 'dark',
          audio_volume: state.savedVolume,
          player_height: state.savedHeight,
        },
      });
    }
    if (path.startsWith('/api/v1/catalog/') && path.endsWith('/playback')) {
      if (options.tools === 'metadata' && state.toolsPreparing)
        return route.fulfill({
          status: 503,
          json: {
            error: {
              code: 'tools_preparing',
              message: 'Waiting for playback tools…',
            },
          },
        });
      if (state.toolsFailed)
        return route.fulfill({
          status: 409,
          json: {
            error: { code: 'conflict', message: 'Playback tools need repair.' },
          },
        });
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
      if (options.tools === 'playback' && state.toolsPreparing)
        return route.fulfill({
          status: 503,
          json: {
            error: {
              code: 'tools_preparing',
              message: 'Waiting for playback tools…',
            },
          },
        });
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
          qualities: options.qualities,
          quality: options.local
            ? undefined
            : input.options.quality === 'auto'
              ? state.autoQuality
              : input.options.quality,
          mode: options.mode ?? 'direct',
          live: options.live ?? false,
          url: options.indexed
            ? '/api/v1/playback/fixture/hls/source/index.m3u8?grant=fixture'
            : options.mode === 'remux'
              ? '/fixture/index.m3u8'
              : '/fixture/video.mp4',
          position: input.position ?? 0,
          duration: 60,
          timeline_start: 0,
          video: true,
          tracks: [],
          subtitles: options.subtitles
            ? [
                {
                  id: 'english',
                  language: 'en',
                  title: 'English',
                  url: '/fixture/subtitles.vtt',
                },
              ]
            : [],
          selected_subtitle: 'off',
          options: input.options,
          probe: {
            streams: [
              { codec_type: 'video', height: 540, avg_frame_rate: '12/1' },
            ],
          },
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
    const indexed =
      /\/playback\/fixture\/(?:hls\/source\/(index|video|audio)\.m3u8|remote\/(video|audio)(-init)?)$/.exec(
        path,
      );
    if (options.indexed && indexed) {
      const [, playlist, track, init] = indexed;
      const type = 'application/vnd.apple.mpegurl';
      if (playlist === 'index') {
        // The codec string the provider declares for its file.
        const avc = original.video.indexOf('avcC') + 5;
        const codec = original.video
          .subarray(avc, avc + 3)
          .toString('hex')
          .toUpperCase();
        return route.fulfill({
          contentType: type,
          body: `#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio",NAME="Original",DEFAULT=YES,AUTOSELECT=YES,URI="audio.m3u8?grant=fixture"\n#EXT-X-STREAM-INF:BANDWIDTH=1500000,CODECS="avc1.${codec},mp4a.40.2",RESOLUTION=960x540,FRAME-RATE=12.000,AUDIO="audio"\nvideo.m3u8?grant=fixture\n`,
        });
      }
      if (playlist)
        return route.fulfill({
          contentType: type,
          body: indexedPlaylist(playlist as 'video' | 'audio'),
        });
      const file = original[track as 'video' | 'audio'];
      const contentType = track === 'video' ? 'video/mp4' : 'audio/mp4';
      if (init)
        return route.fulfill({ contentType, body: fragmentIndex(file).init });
      const [, from, to] = /^bytes=(\d+)-(\d+)$/.exec(
        route.request().headers().range ?? '',
      )!;
      state.ranges.push(`${track}:${from}`);
      if (state.held.test(track, Number(from))) await state.held.promise;
      // A held request may have been abandoned by a seek meanwhile.
      return route
        .fulfill({
          status: 206,
          contentType,
          headers: {
            'Accept-Ranges': 'bytes',
            'Content-Range': `bytes ${from}-${to}/${file.length}`,
          },
          body: file.subarray(Number(from), Number(to) + 1),
        })
        .catch(() => {});
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
          body: options.stalled
            ? '#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:4,\nsegment.ts\n'
            : '#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:60\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:60,\nsegment.ts\n#EXT-X-ENDLIST\n',
        });
      return route.fulfill({
        contentType: url.pathname.endsWith('.ts') ? 'video/mp2t' : 'video/mp4',
        body: url.pathname.endsWith('.ts')
          ? options.stalled
            ? shortSegment
            : segment
          : mp4,
      });
    },
  );
  await page.route('**/fixture/subtitles.vtt', (route) =>
    route.fulfill({
      contentType: 'text/vtt',
      body: 'WEBVTT\n\n00:00:00.000 --> 00:00:59.000\nFixture subtitle\n',
    }),
  );
  await page.route('**/player-test*', (route) =>
    route.fulfill({
      contentType: 'text/html',
      body: `<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1"></head><body style="margin:0;padding:${options.app ? 0 : 16}px"><main id="fixture-root"></main>
      <script type="module">
        import { mount, unmount } from '/node_modules/.vite/deps/svelte.js';
        import Player from '/src/lib/Player.svelte';
        import App from '/src/App.svelte';
        import MusicPlayer from '/src/lib/MusicPlayer.svelte';
        import NativePlayer from '/src/lib/NativePlayer.svelte';
        import { loadAppearance, acceptAppearance } from '/src/lib/appearance.ts';
        import '/src/app.css';
        await loadAppearance('fixture');
        globalThis.__acceptAppearance = acceptAppearance;
        const app = ${
          options.app
            ? "mount(App, { target: document.querySelector('#fixture-root') })"
            : `mount(${options.native ? 'NativePlayer' : options.music ? 'MusicPlayer' : 'Player'}, { target: document.querySelector('#fixture-root'), props: {
          choice: { id: 'fixture', title: ${JSON.stringify(title)}, kind: ${JSON.stringify(options.music ? 'track' : 'movie')}, restore: ${!!options.nativeRestore} }, closed: () => unmount(app, { outro: true })
        }})`
        };
      </script></body></html>`,
    }),
  );
  if (options.native)
    await page.addInitScript((holdTools: boolean) => {
      const bridge = window as unknown as {
        __TAURI_INTERNALS__: unknown;
        __TAURI_EVENT_PLUGIN_INTERNALS__: unknown;
        nativeReady: () => void;
        nativeStarts: number;
        nativeInvocations: number;
      };
      let ready!: () => void;
      const pending = new Promise<void>((resolve) => (ready = resolve));
      let canceled = false;
      let listener: ((event: unknown) => void) | undefined;
      let toolsReady = !holdTools;
      bridge.nativeReady = () => {
        toolsReady = true;
        ready();
        listener?.({
          event: 'mpv-state',
          id: 1,
          payload: {
            status: 'playing',
            media_id: 'fixture',
            title: 'Native fixture',
          },
        });
      };
      bridge.nativeStarts = 0;
      bridge.nativeInvocations = 0;
      bridge.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
      bridge.__TAURI_INTERNALS__ = {
        transformCallback: (callback: (event: unknown) => void) => {
          listener = callback;
          return 1;
        },
        unregisterCallback() {},
        async invoke(
          command: string,
          args: {
            command?: string;
            path?: string;
            method?: string;
            body?: unknown;
          } = {},
        ) {
          if (command === 'backend_request') {
            const response = await fetch(`/api/v1${args.path}`, {
              method: args.method,
              headers: { 'Content-Type': 'application/json' },
              ...(args.body ? { body: JSON.stringify(args.body) } : {}),
            });
            return { status: response.status, body: await response.json() };
          }
          if (command === 'plugin:event|listen') return 1;
          if (command === 'mpv_state')
            return {
              status: 'starting',
              media_id: 'fixture',
              title: 'Native fixture',
            };
          if (command === 'tools_get')
            return {
              tools: holdTools
                ? [
                    {
                      id: 'mpv',
                      preference: {
                        source: 'managed',
                        active: toolsReady ? 'mpv-installed' : null,
                      },
                      selectedPath: toolsReady ? 'mpv.exe' : null,
                    },
                  ]
                : [],
              operation: {
                tool: 'mpv',
                phase: toolsReady ? 'idle' : 'downloading',
                error: null,
              },
            };
          if (command === 'mpv_command' && args.command === 'stop') {
            canceled = true;
            return;
          }
          if (command === 'mpv_play') {
            bridge.nativeInvocations++;
            await pending;
            if (canceled) throw new Error('Playback canceled');
            bridge.nativeStarts++;
            return {
              status: 'playing',
              media_id: 'fixture',
              title: 'Native fixture',
            };
          }
          return null;
        },
      };
    }, !!options.nativeTools);
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
async function revealedPlayer(page: Page) {
  await page
    .getByRole('region', { name: 'Media player' })
    .evaluate(async (element) => {
      await Promise.all(
        element
          .getAnimations({ subtree: true })
          .filter(
            (animation) =>
              animation.effect?.getTiming().iterations !== Infinity,
          )
          .map((animation) => animation.finished.catch(() => {})),
      );
    });
}

test('restored native player keeps its spinner until pending playback starts', async ({
  page,
}, testInfo) => {
  const state = await fixture(page, { native: true, nativeRestore: true });
  await expect(page.getByRole('status')).toContainText('Preparing playback…');
  await testInfo.attach('restored-native-preparation', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  await page.evaluate(() =>
    (window as unknown as { nativeReady: () => void }).nativeReady(),
  );
  await expect(page.getByRole('status')).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { nativeInvocations: number }).nativeInvocations,
    ),
  ).toBe(0);
  expect(state.errors).toEqual([]);
});

for (const tools of ['metadata', 'playback'] as const) {
  test(`player waits for tools during ${tools} and starts automatically`, async ({
    page,
  }, testInfo) => {
    const state = await fixture(page, { tools });
    await expect(page.getByRole('status')).toContainText(
      'Waiting for playback tools…',
    );
    await expect(page.locator('.loading-spinner')).toBeVisible();
    expect(state.starts).toEqual([]);
    await revealedPlayer(page);
    await testInfo.attach('waiting-for-tools', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    state.toolsPreparing = false;
    await decoded(page);
    await expect.poll(() => state.starts.length).toBe(1);
    expect(state.errors).toEqual([]);
    await testInfo.attach('automatic-playback', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
  });
}

test('closing a player waiting for tools cancels its pending playback', async ({
  page,
}, testInfo) => {
  const state = await fixture(page, { tools: 'metadata' });
  await expect(page.getByRole('status')).toContainText(
    'Waiting for playback tools…',
  );
  await page.getByRole('button', { name: 'Close player' }).click();
  await expect(page.getByRole('region', { name: 'Media player' })).toHaveCount(
    0,
  );
  state.toolsPreparing = false;
  await page.waitForTimeout(1500);
  expect(state.starts).toEqual([]);
  expect(state.errors).toEqual([]);
  await testInfo.attach('closed-without-playback', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('choosing another item while waiting only starts the latest item', async ({
  page,
}, testInfo) => {
  const state = await fixture(page, { tools: 'metadata', app: true });
  await page
    .getByRole('button', { name: 'Watch Channel 0', exact: true })
    .click();
  await expect(
    page.getByRole('status').filter({ hasText: 'Waiting for playback tools…' }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Watch Channel 1', exact: true })
    .click();
  state.toolsPreparing = false;
  await decoded(page);
  expect(state.starts.map((item) => item.media_id)).toEqual(['twitch:1']);
  expect(state.errors).toEqual([]);
  await testInfo.attach('latest-item-playing', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('a YouTube link opens a cancelable player while its tools are preparing', async ({
  page,
}, testInfo) => {
  const state = await fixture(page, { app: true });
  state.resolvePreparing = true;
  await expect(
    page.getByRole('button', { name: 'Watch Channel 0', exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    const link = document.createElement('a');
    link.href = 'https://www.youtube.com/watch?v=abcdefghijk';
    link.textContent = 'Linked video';
    document.body.append(link);
    link.click();
  });
  await expect(
    page.getByRole('region', { name: 'Media player' }),
  ).toBeVisible();
  await expect(
    page.getByRole('status').filter({ hasText: 'Waiting for playback tools…' }),
  ).toBeVisible();
  await revealedPlayer(page);
  await testInfo.attach('linked-video-waiting', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  await page.getByRole('button', { name: 'Close player' }).click();
  state.resolvePreparing = false;
  await expect(page.getByRole('region', { name: 'Media player' })).toHaveCount(
    0,
  );
  await page.waitForTimeout(1500);
  expect(state.starts).toEqual([]);
  expect(state.errors).toEqual([]);
});

test('failed tool preparation replaces the spinner with a retryable error', async ({
  page,
}, testInfo) => {
  const state = await fixture(page, { tools: 'metadata' });
  await expect(page.getByRole('status')).toContainText(
    'Waiting for playback tools…',
  );
  state.toolsPreparing = false;
  state.toolsFailed = true;
  await expect(page.getByRole('alert')).toContainText(
    'Playback tools need repair.',
  );
  await expect(page.getByRole('status')).toHaveCount(0);
  await testInfo.attach('tool-failure', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  state.toolsFailed = false;
  await page.getByRole('button', { name: 'Retry playback' }).click();
  await decoded(page);
  expect(state.errors).toEqual([]);
});

test('music waits for tools and starts its streaming fallback automatically', async ({
  page,
}, testInfo) => {
  const state = await fixture(page, { tools: 'metadata', music: true });
  await expect(page.getByRole('status')).toContainText(
    'Waiting for playback tools…',
  );
  await expect(page.getByRole('status').locator('svg')).toBeVisible();
  await testInfo.attach('music-waiting-for-tools', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  state.toolsPreparing = false;
  await decoded(page);
  expect(state.errors).toEqual([]);
});

for (const close of [false, true]) {
  test(`native player ${close ? 'cancels' : 'automatically starts'} pending preparation`, async ({
    page,
  }, testInfo) => {
    const state = await fixture(page, { native: true });
    await expect(page.getByRole('status')).toContainText('Preparing playback…');
    await expect(page.getByRole('status').locator('svg')).toBeVisible();
    await testInfo.attach('native-preparation', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    if (close) await page.getByRole('button', { name: 'Close player' }).click();
    await page.evaluate(() =>
      (window as unknown as { nativeReady: () => void }).nativeReady(),
    );
    await expect(page.getByRole('status')).toHaveCount(0);
    expect(
      await page.evaluate(
        () => (window as unknown as { nativeStarts: number }).nativeStarts,
      ),
    ).toBe(close ? 0 : 1);
    expect(state.errors).toEqual([]);
    await testInfo.attach('native-preparation-finished', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
  });
}

for (const close of [false, true]) {
  test(`native MPV installation ${close ? 'can be canceled' : 'automatically resumes playback'}`, async ({
    page,
  }, testInfo) => {
    const state = await fixture(page, { native: true, nativeTools: true });
    await expect(page.getByRole('status')).toContainText(
      'Waiting for playback tools…',
    );
    expect(
      await page.evaluate(
        () =>
          (window as unknown as { nativeInvocations: number })
            .nativeInvocations,
      ),
    ).toBe(0);
    await testInfo.attach('waiting-for-mpv', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    if (close) await page.getByRole('button', { name: 'Close player' }).click();
    await page.evaluate(() =>
      (window as unknown as { nativeReady: () => void }).nativeReady(),
    );
    await expect(page.getByRole('status')).toHaveCount(0);
    expect(
      await page.evaluate(
        () => (window as unknown as { nativeStarts: number }).nativeStarts,
      ),
    ).toBe(close ? 0 : 1);
    expect(state.errors).toEqual([]);
    await testInfo.attach('mpv-installation-finished', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
  });
}

test('web playback records active time but excludes paused time and seeking', async ({
  page,
}) => {
  const state = await fixture(page);
  await decoded(page);
  await expect
    .poll(() =>
      page
        .locator('video')
        .evaluate((video: HTMLVideoElement) => video.currentTime),
    )
    .toBeGreaterThan(0.5);
  await page
    .locator('video')
    .evaluate((video: HTMLVideoElement) => video.pause());
  await expect.poll(() => state.progress.at(-1)?.state).toBe('paused');
  const seconds = state.progress.at(-1)!.active_seconds;
  expect(seconds).toBeGreaterThan(0);
  const position = await page
    .locator('video')
    .evaluate((video: HTMLVideoElement) => video.currentTime);
  expect(seconds).toBeLessThanOrEqual(position + 1);
  await page.locator('video').evaluate(async (video: HTMLVideoElement) => {
    await new Promise<void>((resolve) => {
      video.addEventListener('seeked', () => resolve(), { once: true });
      video.currentTime = 45;
    });
  });
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await expect.poll(() => state.progress.at(-1)?.state).toBe('stopped');
  expect(state.progress.at(-1)!.active_seconds).toBe(seconds);
  expect(state.progress.at(-1)!.position).toBeGreaterThanOrEqual(45);
  expect(state.errors).toEqual([]);
});

test('opening and replacing web playback focuses the player for immediate shortcuts', async ({
  page,
}, info) => {
  const state = await fixture(page, { app: true, hold: true });
  const player = page.getByRole('region', { name: 'Media player' });
  const video = page.locator('video');
  await page
    .getByRole('button', { name: 'Watch Channel 0', exact: true })
    .click();
  await expect(player).toBeFocused();
  await page.keyboard.press('m');
  await expect(video).toHaveJSProperty('muted', true);

  state.metadata.release();
  state.preparation.release();
  state.media.release();
  await decoded(page);
  await page.keyboard.press('Space');
  await expect(video).toHaveJSProperty('paused', true);
  await page.keyboard.press('k');
  await expect(video).toHaveJSProperty('paused', false);

  await page
    .getByRole('button', { name: 'Watch Channel 1', exact: true })
    .click();
  await expect(player).toBeFocused();
  await expect.poll(() => state.starts.length).toBe(2);
  await decoded(page);
  await page.keyboard.press('m');
  await expect(video).toHaveJSProperty('muted', false);
  await page.screenshot({ path: info.outputPath('opened-player-focus.png') });
  expect(state.errors).toEqual([]);
});

test('web player fills the workspace edges and resizes by pointer and keyboard', async ({
  page,
}) => {
  const state = await fixture(page, { app: true, playerHeight: 330 });
  await page
    .getByRole('button', { name: 'Watch Channel 0', exact: true })
    .click();
  await decoded(page);
  const player = page.getByRole('region', { name: 'Media player' });
  const handle = page.getByRole('slider', { name: 'Resize player height' });
  await expect(player).toHaveCSS('transform', 'none');
  const edges = await player.evaluate((element) => {
    const box = element.getBoundingClientRect();
    const workspace = element.closest('.workspace-scroll')!;
    const bounds = workspace.getBoundingClientRect();
    return {
      left: box.left - bounds.left,
      top: box.top - bounds.top,
      right: bounds.left + workspace.clientWidth - box.right,
    };
  });
  expect(Math.abs(edges.left)).toBeLessThanOrEqual(1);
  expect(Math.abs(edges.top)).toBeLessThanOrEqual(1);
  expect(Math.abs(edges.right)).toBeLessThanOrEqual(1);
  const initial = (await player.boundingBox())!.height;
  expect(initial).toBeCloseTo(330, 0);
  const grip = (await handle.boundingBox())!;
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(
    grip.x + grip.width / 2,
    grip.y + grip.height / 2 - 100,
    { steps: 8 },
  );
  await page.mouse.up();
  await expect
    .poll(async () => (await player.boundingBox())!.height)
    .toBeCloseTo(initial - 100, 0);
  await expect.poll(() => state.savedHeight).toBeCloseTo(initial - 100, 0);
  await handle.focus();
  await page.keyboard.press('ArrowDown');
  await expect
    .poll(async () => (await player.boundingBox())!.height)
    .toBeCloseTo(initial - 90, 0);
  await expect.poll(() => state.savedHeight).toBeCloseTo(initial - 90, 0);
  await page.keyboard.press('Home');
  await expect(player).toHaveCSS('height', '210px');
  await expect.poll(() => state.savedHeight).toBe(210);
  await page.keyboard.press('Enter');
  await expect
    .poll(() => player.evaluate((element) => element.style.height))
    .toBe('');
  await expect.poll(() => state.savedHeight).toBeNull();
  expect(state.errors).toEqual([]);
});

test('open web player follows remote per-user height changes', async ({
  page,
}) => {
  const state = await fixture(page, { app: true, playerHeight: 330 });
  await page
    .getByRole('button', { name: 'Watch Channel 0', exact: true })
    .click();
  await decoded(page);
  const player = page.getByRole('region', { name: 'Media player' });
  await expect
    .poll(async () => (await player.boundingBox())!.height)
    .toBeCloseTo(330, 0);
  await page.evaluate(() => {
    (
      globalThis as typeof globalThis & {
        __acceptAppearance: (value: { player_height: number | null }) => void;
      }
    ).__acceptAppearance({ player_height: 280 });
  });
  await expect
    .poll(async () => (await player.boundingBox())!.height)
    .toBeCloseTo(280, 0);
  await page.evaluate(() => {
    (
      globalThis as typeof globalThis & {
        __acceptAppearance: (value: { player_height: number | null }) => void;
      }
    ).__acceptAppearance({ player_height: null });
  });
  await expect
    .poll(() => player.evaluate((element) => element.style.height))
    .toBe('');
  expect(state.errors).toEqual([]);
});

test('saved height does not resize fixed player instances', async ({
  page,
}) => {
  const state = await fixture(page, { playerHeight: 330 });
  await decoded(page);
  const player = page.getByRole('region', { name: 'Media player' });
  await expect
    .poll(() => player.evaluate((element) => element.style.height))
    .toBe('');
  await expect(
    page.getByRole('slider', { name: 'Resize player height' }),
  ).toHaveCount(0);
  expect(state.savedHeight).toBe(330);
  expect(state.errors).toEqual([]);
});

test('sidebar controls stay aligned through collapse, expansion, and reversal', async ({
  page,
}) => {
  const state = await fixture(page, { app: true });
  await page.getByRole('button', { name: 'Collapse sidebar' }).waitFor();
  const sidebar = page.locator('.primary-sidebar');
  await page.evaluate(() => {
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (keyframes, options) {
      const animation = animate.call(this, keyframes, options);
      if (this.closest('.primary-sidebar')) animation.pause();
      return animation;
    };
  });
  async function toggle(name: string) {
    await sidebar.evaluate(async (element, name) => {
      element
        .querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`)!
        .click();
      await new Promise(requestAnimationFrame);
      for (const animation of element.getAnimations({ subtree: true })) {
        animation.pause();
        animation.currentTime = 0;
      }
    }, name);
  }
  async function seek(time: number) {
    await sidebar.evaluate((element, time) => {
      for (const animation of element.getAnimations({ subtree: true })) {
        animation.currentTime = time;
      }
    }, time);
  }
  async function finish() {
    await sidebar.evaluate((element) => {
      for (const animation of element.getAnimations({ subtree: true }))
        animation.finish();
    });
  }
  async function snapshot() {
    return sidebar.evaluate((element) => {
      const box = (e: Element) => e.getBoundingClientRect().toJSON();
      const label = element.querySelector('.profile-label')!;
      const selected = element.querySelector(
        '.primary-navigation [aria-current="page"]',
      )!;
      const rail = selected.querySelector('[data-nav-accent]')!;
      const railBox = box(rail);
      return {
        surface: box(element.querySelector('.sidebar-surface')!),
        label: box(label),
        opacity: Number(getComputedStyle(label).opacity),
        rail: railBox,
        railVisible: document
          .elementsFromPoint(
            railBox.x + railBox.width / 2,
            railBox.y + railBox.height / 2,
          )
          .includes(selected),
      };
    });
  }
  const expanded = await snapshot();
  function aligned(frame: Awaited<ReturnType<typeof snapshot>>) {
    // Check painted controls and their hit areas together, not just endpoints.
    expect(frame.label).toEqual(expanded.label);
    expect(frame.rail.right).toBeCloseTo(frame.surface.right, 1);
    expect(frame.rail.width).toBeCloseTo(expanded.rail.width, 1);
    expect(frame.railVisible).toBe(true);
  }
  for (const direction of ['Collapse', 'Expand']) {
    await toggle(`${direction} sidebar`);
    for (const time of [0, 25, 50, 100, 150, 200]) {
      await seek(time);
      const frame = await snapshot();
      aligned(frame);
      if (time === 50) {
        expect(frame.opacity).toBeGreaterThan(0);
        expect(frame.opacity).toBeLessThan(1);
        await page.screenshot({
          path: `.local/player-ui/sidebar-${direction.toLowerCase()}.png`,
        });
      }
    }
    await finish();
    const end = await snapshot();
    expect(end.opacity).toBe(direction === 'Collapse' ? 0 : 1);
    if (direction === 'Collapse') {
      await page.screenshot({ path: '.local/player-ui/sidebar-collapsed.png' });
    }
  }
  await toggle('Collapse sidebar');
  await seek(50);
  const interrupted = await snapshot();
  await toggle('Expand sidebar');
  const reversed = await snapshot();
  expect(reversed.surface.width).toBeCloseTo(interrupted.surface.width, 1);
  for (const time of [0, 50, 100, 200]) {
    await seek(time);
    aligned(await snapshot());
  }
  await finish();
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.getByRole('button', { name: 'Collapse sidebar' }).click();
  await expect(sidebar).toHaveCSS('width', '72px');
  const reduced = await snapshot();
  aligned(reduced);
  // Preserve scrolling on shorter windows while the navigation clip resizes.
  await page.setViewportSize({ width: 1000, height: 480 });
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  const navigation = page.getByRole('navigation', { name: 'Main navigation' });
  const scrollTop = await navigation.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
    return element.scrollTop;
  });
  expect(scrollTop).toBeGreaterThan(0);
  await toggle('Expand sidebar');
  for (const time of [0, 50, 100, 200]) {
    await seek(time);
    const frame = await snapshot();
    expect(frame.rail.right).toBeCloseTo(frame.surface.right, 1);
    expect(frame.railVisible).toBe(true);
    expect(await navigation.evaluate((element) => element.scrollTop)).toBe(
      scrollTop,
    );
  }
  await finish();
  expect(state.errors).toEqual([]);
});
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
    .getByRole('menuitemradio', { name: 'Original (540p)', exact: true })
    .click();
  await expect.poll(() => state.starts.length).toBe(2);
  expect(state.starts[1].options.quality).toBe('original');
  expect(state.starts[1].position).toBeGreaterThanOrEqual(8);
  await decoded(page);
  await expect(volume).toHaveValue('0.23');
  await page.getByRole('button', { name: 'Quality', exact: true }).click();
  await expect(
    page.getByRole('menuitemradio', { name: 'Original (540p)', exact: true }),
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
  await expect(page.locator('.primary-sidebar')).toBeHidden();
  const playerWidth = (await page.locator('.player').boundingBox())!.width;
  const outsidePlayer = page.viewportSize()!.width - playerWidth;
  await page.setViewportSize({ width: outsidePlayer + 380, height: 844 });
  await expect(page.locator('.player-volume-range')).toBeHidden();
  await expect(page.locator('.playback-mode')).toBeHidden();
  await page.setViewportSize({ width: outsidePlayer + 381, height: 844 });
  // Narrow windows with a mouse keep the player volume; only touch devices hide it.
  await expect(page.locator('.player-volume-range')).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await insidePlayer(page);
  // Changing quality preserves the pause established before seeking.
  await expect(
    page.getByRole('button', { name: 'Play', exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: '.local/player-ui/mobile.png' });
  expect(state.errors).toEqual([]);
});

test('fullscreen shortcut exits while the entry event is still pending', async ({
  page,
}, testInfo) => {
  await fixture(page);
  await decoded(page);
  await page.evaluate(() => {
    document.addEventListener(
      'fullscreenchange',
      (event) => {
        if (document.fullscreenElement) event.stopImmediatePropagation();
      },
      { capture: true },
    );
  });
  await page.getByRole('region', { name: 'Media player' }).focus();
  await page.keyboard.press('f');
  await expect
    .poll(() => page.evaluate(() => document.fullscreenElement !== null))
    .toBe(true);
  await page.keyboard.press('f');
  await expect
    .poll(() => page.evaluate(() => document.fullscreenElement === null))
    .toBe(true);
  await testInfo.attach('fullscreen-shortcut', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('web player supports YouTube playback shortcuts and left/right-click pause', async ({
  page,
}, info) => {
  const state = await fixture(page, { subtitles: true });
  await decoded(page);
  const player = page.getByRole('region', { name: 'Media player' });
  const video = page.locator('video');

  // A mouse reveals the controls by hovering, so its click still pauses after
  // they hide instead of only revealing them like a tap.
  await video.hover({ position: { x: 100, y: 100 } });
  await expect(page.locator('.player-controls')).toHaveCSS('opacity', '0', {
    timeout: 5000,
  });
  await page.mouse.down();
  await page.mouse.up();
  await expect(player).toBeFocused();
  await expect
    .poll(() => video.evaluate((element) => element.paused))
    .toBe(true);
  await video.click({ position: { x: 100, y: 100 } });
  await expect
    .poll(() => video.evaluate((element) => element.paused))
    .toBe(false);

  await page.keyboard.press('k');
  await expect
    .poll(() => video.evaluate((element) => element.paused))
    .toBe(true);
  await page.keyboard.press('Space');
  await expect
    .poll(() => video.evaluate((element) => element.paused))
    .toBe(false);
  await video.click({ button: 'right', position: { x: 100, y: 100 } });
  await expect
    .poll(() => video.evaluate((element) => element.paused))
    .toBe(true);
  await video.click({ button: 'right', position: { x: 100, y: 100 } });
  await expect
    .poll(() => video.evaluate((element) => element.paused))
    .toBe(false);
  await video.click({ button: 'right', position: { x: 100, y: 100 } });
  await expect
    .poll(() => video.evaluate((element) => element.paused))
    .toBe(true);

  await video.evaluate((element) => {
    element.currentTime = 20;
    element.dispatchEvent(new Event('timeupdate'));
  });
  await player.focus();
  for (const [key, expected] of [
    ['ArrowLeft', 15],
    ['ArrowRight', 20],
    ['j', 10],
    ['l', 20],
  ] as const) {
    await page.keyboard.press(key);
    await expect
      .poll(() => video.evaluate((element) => element.currentTime))
      .toBeCloseTo(expected, 1);
  }

  await page.keyboard.press('ArrowUp');
  await expect
    .poll(() => video.evaluate((element) => element.volume))
    .toBeCloseTo(0.36, 2);
  await page.keyboard.press('ArrowDown');
  await expect
    .poll(() => video.evaluate((element) => element.volume))
    .toBeCloseTo(0.31, 2);
  await page.keyboard.press('m');
  expect(await video.evaluate((element) => element.muted)).toBe(true);
  await expect(player).toBeFocused();
  await expect(player).not.toHaveCSS('outline-style', 'none');

  await page.keyboard.press('f');
  await expect
    .poll(() =>
      page.evaluate(() =>
        document.fullscreenElement?.classList.contains('player'),
      ),
    )
    .toBe(true);
  await expect(player).toBeFocused();
  await expect(player).toHaveCSS('outline-style', 'none');
  await page.keyboard.press('m');
  expect(await video.evaluate((element) => element.muted)).toBe(false);
  await expect(player).toHaveCSS('outline-style', 'none');
  await page.screenshot({ path: info.outputPath('fullscreen-keyboard.png') });
  await page.keyboard.press('f');
  await expect
    .poll(() => page.evaluate(() => document.fullscreenElement))
    .toBeNull();
  await expect(player).toBeFocused();
  await expect(player).not.toHaveCSS('outline-style', 'none');

  await expect
    .poll(() => video.evaluate((element) => element.textTracks.length))
    .toBe(1);
  await page.keyboard.press('c');
  await expect
    .poll(() => video.evaluate((element) => element.textTracks[0]?.mode))
    .toBe('showing');
  await page.keyboard.press('c');
  await expect
    .poll(() => video.evaluate((element) => element.textTracks[0]?.mode))
    .toBe('disabled');

  const beforeStep = await video.evaluate((element) => element.currentTime);
  await page.keyboard.press('.');
  await expect
    .poll(() => video.evaluate((element) => element.currentTime))
    .toBeCloseTo(beforeStep + 1 / 12, 2);
  await page.keyboard.press(',');
  await expect
    .poll(() => video.evaluate((element) => element.currentTime))
    .toBeCloseTo(beforeStep, 2);

  await page.keyboard.press('Shift+Period');
  expect(await video.evaluate((element) => element.playbackRate)).toBe(1.25);
  await page.keyboard.press('Shift+Comma');
  expect(await video.evaluate((element) => element.playbackRate)).toBe(1);
  expect(state.errors).toEqual([]);
});

test.describe('touch video playback', () => {
  test.use({ hasTouch: true, viewport: { width: 390, height: 844 } });

  test('repeated side taps accumulate during a slow transcoded seek and keep paused playback paused', async ({
    page,
  }, testInfo) => {
    await page.clock.install();
    const state = await fixture(page, { mode: 'remux' });
    await decoded(page);
    const video = page.locator('video');
    await page.getByRole('button', { name: 'Pause', exact: true }).tap();
    const position = page.getByRole('slider', { name: 'Playback position' });
    await position.fill('20');
    await expect
      .poll(async () => Number(await position.inputValue()))
      .toBeCloseTo(20, 0);
    await expect
      .poll(() =>
        video.evaluate((element: HTMLVideoElement) => element.readyState),
      )
      .toBeGreaterThanOrEqual(2);
    await page.route('**/api/v1/playback/*/seek', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 500));
      await route.fallback();
    });
    await freezeClock(page);
    for (let tap = 0; tap < 3; tap++) {
      await video.tap({ position: { x: 340, y: 95 } });
    }
    await page.clock.resume();
    await expect
      .poll(async () => Number(await position.inputValue()))
      .toBeCloseTo(40, 0);
    await expect
      .poll(() =>
        video.evaluate((element: HTMLVideoElement) => element.readyState),
      )
      .toBeGreaterThanOrEqual(2);
    await expect(video).toHaveJSProperty('paused', true);
    await page.screenshot({
      path: testInfo.outputPath('mobile-transcoded-seek.png'),
    });
    expect(state.errors).toEqual([]);
  });

  test('tapping the center reveals hidden controls before toggling playback', async ({
    page,
  }, testInfo) => {
    const state = await fixture(page);
    await decoded(page);
    const video = page.locator('video');
    const controls = page.locator('.player-controls');

    await expect(controls).toHaveCSS('opacity', '0', { timeout: 5000 });
    await video.tap({ position: { x: 195, y: 100 } });
    await expect(controls).toHaveCSS('opacity', '1');
    await expect(video).toHaveJSProperty('paused', false);
    await video.tap({ position: { x: 195, y: 100 } });
    await expect(video).toHaveJSProperty('paused', true);
    await expect(
      page.getByRole('button', { name: 'Play', exact: true }),
    ).toBeVisible();
    await expect(controls).toHaveCSS('opacity', '1');
    await testInfo.attach('Paused touch video', {
      body: await page.screenshot({
        path: testInfo.outputPath('tap-paused.png'),
      }),
      contentType: 'image/png',
    });
    await video.tap({ position: { x: 195, y: 100 } });
    await expect(video).toHaveJSProperty('paused', false);

    await page.getByRole('button', { name: 'Pause', exact: true }).tap();
    await expect(video).toHaveJSProperty('paused', true);
    await page.getByRole('button', { name: 'Play', exact: true }).tap();
    await expect(video).toHaveJSProperty('paused', false);
    await expect(page.locator('.volume-controls')).toHaveCount(0);
    await expect(video).toHaveJSProperty('volume', 1);
    await expect(video).toHaveJSProperty('muted', false);
    await expect(video).toHaveJSProperty('paused', false);
    // Mobile browsers leave :hover on the controls after tapping one.
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('DOM.enable');
    await cdp.send('CSS.enable');
    const { root } = await cdp.send('DOM.getDocument');
    const { nodeId } = await cdp.send('DOM.querySelector', {
      nodeId: root.nodeId,
      selector: '.player-controls',
    });
    await cdp.send('CSS.forcePseudoState', {
      nodeId,
      forcedPseudoClasses: ['hover'],
    });
    await expect(page.locator('.player-controls:hover')).toHaveCount(1);
    await expect(controls).toHaveCSS('opacity', '0', { timeout: 5000 });
    await video.tap({ position: { x: 195, y: 100 } });
    await expect(controls).toHaveCSS('opacity', '1');
    await expect(video).toHaveJSProperty('paused', false);
    expect(state.errors).toEqual([]);
  });
});

test('online originals play as byte-range HLS and seek within the client', async ({
  page,
}) => {
  const state = await fixture(page, { indexed: true });
  await decoded(page);
  expect(state.ranges.some((range) => range.startsWith('audio:'))).toBe(true);
  const position = page.getByRole('slider', { name: 'Playback position' });
  await position.fill('42');
  await expect
    .poll(async () => Number(await position.inputValue()))
    .toBeGreaterThanOrEqual(42);
  await decoded(page);
  // Only the fragment containing the target is requested; the server never restarts.
  let start = 0;
  const target = fragmentIndex(original.video).fragments.find((fragment) => {
    start += fragment.duration;
    return start > 42;
  })!;
  expect(state.ranges).toContain(`video:${target.offset}`);
  expect(state.seeks).toEqual([]);
  expect(state.starts).toHaveLength(1);
  expect(state.errors).toEqual([]);
});

test('a seek near the end of an unbuffered fragment waits for the next one before resuming', async ({
  page,
}) => {
  const state = await fixture(page, { indexed: true, hold: true });
  const { fragments } = fragmentIndex(original.video);
  // Two-second fragments: the target is 0.5 s before the end of the one at 40 s.
  const starts = fragments.map((_, index) =>
    fragments.slice(0, index).reduce((sum, f) => sum + f.duration, 0),
  );
  const target = fragments.findIndex((_, index) => starts[index] >= 40);
  const seekTo = starts[target + 1] - 0.5;
  // Initial buffering stops at 20 s; only the target fragment is delivered.
  state.held.test = (track, offset) =>
    track === 'video' &&
    offset >= fragments[10].offset &&
    offset !== fragments[target].offset;
  state.metadata.release();
  state.preparation.release();
  state.media.release();
  await decoded(page);
  const video = page.locator('video');
  await page
    .getByRole('slider', { name: 'Playback position' })
    .fill(String(seekTo));
  await expect
    .poll(() =>
      video.evaluate(
        (v: HTMLVideoElement, at) =>
          Array.from({ length: v.buffered.length }).some(
            (_, i) => v.buffered.start(i) <= at && at < v.buffered.end(i),
          ),
        seekTo,
      ),
    )
    .toBe(true);
  // With half a second buffered, playback waits instead of starting and stalling.
  await page.waitForTimeout(1500);
  expect(
    await video.evaluate((v: HTMLVideoElement) => v.currentTime),
  ).toBeCloseTo(seekTo, 1);
  await expect(page.getByRole('status')).toContainText('Buffering');
  await expect(
    page.getByRole('button', { name: 'Pause', exact: true }),
  ).toBeVisible();
  state.held.release();
  await expect
    .poll(() => video.evaluate((v: HTMLVideoElement) => v.currentTime))
    .toBeGreaterThan(seekTo + 1);
  await expect(page.getByRole('status')).toHaveCount(0);
  expect(state.seeks).toEqual([]);
  expect(state.errors).toEqual([]);
});

test('pausing while a seek waits for media keeps playback paused', async ({
  page,
}) => {
  const state = await fixture(page, { indexed: true, hold: true });
  const { fragments } = fragmentIndex(original.video);
  state.held.test = (track, offset) =>
    track === 'video' && offset >= fragments[10].offset;
  state.metadata.release();
  state.preparation.release();
  state.media.release();
  await decoded(page);
  await page.getByRole('slider', { name: 'Playback position' }).fill('45');
  await expect(page.getByRole('status')).toContainText('Buffering');
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Play', exact: true }),
  ).toBeVisible();
  state.held.release();
  await expect(page.getByRole('status')).toHaveCount(0);
  await page.waitForTimeout(500);
  const video = page.locator('video');
  await expect(video).toHaveJSProperty('paused', true);
  expect(
    await video.evaluate((v: HTMLVideoElement) => v.currentTime),
  ).toBeCloseTo(45, 0);
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

test('the player and its controls follow sidebar resizing without clipping or stretching controls', async ({
  page,
}) => {
  const state = await fixture(page, { app: true });
  await page
    .getByRole('button', { name: 'Watch Channel 0', exact: true })
    .click();
  await decoded(page);
  await expect(page.locator('.player')).toHaveCSS('transform', 'none');
  await page.evaluate(() => {
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (keyframes, options) {
      const animation = animate.call(this, keyframes, options);
      if (this.hasAttribute('data-sidebar-resize')) animation.pause();
      return animation;
    };
  });
  async function sample(time?: number) {
    return page.evaluate((time) => {
      if (time !== undefined) {
        const animations = Reflect.get(
          window,
          'sidebarTestAnimations',
        ) as Animation[];
        for (const animation of animations) animation.currentTime = time;
      }
      const box = (element: Element) =>
        element.getBoundingClientRect().toJSON();
      const player = document.querySelector('.player')!;
      const bounds = box(player);
      const close = player.querySelector('[aria-label="Close player"]')!;
      const button = box(close);
      const video = player.querySelector('video')!;
      const videoBox = box(video);
      const videoStyle = getComputedStyle(video);
      return {
        player: bounds,
        feed: box(document.querySelector('[data-feed-content]')!),
        card: box(
          document.querySelector(
            '[data-feed-content] [data-card-grid] [data-layout-key]',
          )!,
        ),
        sidebar: box(document.querySelector('.sidebar-surface')!),
        close: button,
        videoScaleX: videoBox.width / parseFloat(videoStyle.width),
        videoScaleY: videoBox.height / parseFloat(videoStyle.height),
        closeVisible: document
          .elementsFromPoint(
            button.x + button.width / 2,
            button.y + button.height / 2,
          )
          .some((e) => close.contains(e)),
        leftVisible: document
          .elementsFromPoint(bounds.x + 2, bounds.y + bounds.height / 2)
          .some((e) => player.contains(e)),
        controls: [
          ...player.querySelectorAll('.player-button, .option-button'),
        ].map(box),
      };
    }, time);
  }
  for (const [width, section] of [
    [1280, 'Twitch'],
    [1000, 'Twitch'],
    [1280, 'YouTube'],
    [1280, 'Kick'],
  ] as const) {
    await page.setViewportSize({ width, height: 900 });
    await page.getByRole('link', { name: section, exact: true }).click();
    await expect(page.locator('[data-page-content]')).toHaveAttribute(
      'data-page-phase',
      'idle',
    );
    if (section === 'YouTube')
      await page
        .getByRole('button', { name: 'Open watchlists', exact: true })
        .click();
    await page
      .locator('[data-feed-content] [data-layout-key]')
      .first()
      .waitFor();
    await page.evaluate(
      () =>
        new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        ),
    );
    for (const direction of ['Collapse', 'Expand']) {
      await page.locator('.player').dispatchEvent('pointermove');
      const before = await sample();
      await page.getByRole('button', { name: `${direction} sidebar` }).click();
      await page.evaluate(() => {
        Reflect.set(
          window,
          'sidebarTestAnimations',
          document
            .getAnimations()
            .filter((a) =>
              (a.effect as KeyframeEffect).target?.hasAttribute(
                'data-sidebar-resize',
              ),
            ),
        );
      });
      const after = await sample(200);
      expect(Math.abs(after.player.x - before.player.x)).toBeCloseTo(108, 1);
      for (const time of [0, 25, 50, 100, 150]) {
        const frame = await sample(time);
        const progress =
          (frame.sidebar.width - before.sidebar.width) /
          (after.sidebar.width - before.sidebar.width);
        for (const axis of ['x', 'y', 'width', 'height']) {
          expect(
            frame.player[axis],
            `${direction} at ${width}px, ${time}ms: player ${axis}`,
          ).toBeCloseTo(
            before.player[axis] +
              (after.player[axis] - before.player[axis]) * progress,
            0,
          );
          for (const part of ['feed', 'card'] as const) {
            const expected =
              before[part][axis] +
              (after[part][axis] - before[part][axis]) * progress;
            expect(
              Math.abs(frame[part][axis] - expected),
              `${section} ${direction} at ${width}px, ${time}ms: ${part} ${axis}`,
            ).toBeLessThan(
              // Card grids also interpolate their fixed padding and gaps.
              part === 'card' ? 2 : 0.5,
            );
          }
        }
        for (const control of frame.controls) {
          expect(control.width).toBeCloseTo(36, 0);
          expect(control.height).toBeCloseTo(36, 0);
        }
        expect(frame.videoScaleX).toBeCloseTo(frame.videoScaleY, 3);
        expect(frame.closeVisible).toBe(true);
        expect(frame.leftVisible).toBe(true);
        if (time === 50)
          await page.screenshot({
            path: `.local/player-ui/player-sidebar-${section}-${width}-${direction.toLowerCase()}.png`,
          });
      }
      await page.evaluate(() => {
        for (const animation of Reflect.get(
          window,
          'sidebarTestAnimations',
        ) as Animation[])
          animation.finish();
      });
    }
  }
  expect(state.errors).toEqual([]);
});

test('player loading and error messages stay centered during sidebar resizing', async ({
  page,
}) => {
  const state = await fixture(page, { app: true, hold: true, fail: true });
  await page
    .getByRole('button', { name: 'Watch Channel 0', exact: true })
    .click();
  await expect(page.locator('.player')).toHaveCSS('transform', 'none');
  await page.evaluate(() => {
    const animate = Element.prototype.animate;
    const animations: Animation[] = [];
    Reflect.set(window, 'statusTestAnimations', animations);
    Element.prototype.animate = function (keyframes, options) {
      const animation = animate.call(this, keyframes, options);
      if (this.hasAttribute('data-sidebar-resize')) {
        animation.pause();
        animations.push(animation);
      }
      return animation;
    };
  });
  for (const phase of ['loading', 'error']) {
    if (phase === 'error') {
      state.metadata.release();
      state.preparation.release();
      await expect(page.getByRole('alert')).toContainText(
        'The stream is temporarily unavailable',
      );
    } else await expect(page.getByRole('status')).toBeVisible();
    for (const width of [1280, 1000]) {
      await page.setViewportSize({ width, height: 900 });
      await page.evaluate(
        () =>
          new Promise((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(resolve)),
          ),
      );
      for (const direction of ['Collapse', 'Expand']) {
        await page
          .getByRole('button', { name: `${direction} sidebar` })
          .click();
        for (const time of [0, 25, 50, 100, 200]) {
          const frame = await page.evaluate((time) => {
            for (const animation of Reflect.get(
              window,
              'statusTestAnimations',
            ) as Animation[])
              animation.currentTime = time;
            const box = (element: Element) =>
              element.getBoundingClientRect().toJSON();
            const status = document.querySelector('.player-status')!;
            const retry = status.querySelector('button');
            const retryBox = retry ? box(retry) : null;
            return {
              player: box(document.querySelector('.player')!),
              status: box(status),
              children: [...status.children].map(box),
              spinnerRunning: status
                .querySelector('.loading-spinner')
                ?.getAnimations()
                .some((animation) => animation.playState === 'running'),
              retryClickable:
                retryBox &&
                document
                  .elementsFromPoint(
                    retryBox.x + retryBox.width / 2,
                    retryBox.y + retryBox.height / 2,
                  )
                  .includes(retry!),
            };
          }, time);
          const centerX = frame.player.x + frame.player.width / 2;
          const centerY = frame.player.y + frame.player.height / 2 - 12;
          expect(
            frame.status.x + frame.status.width / 2,
            `${phase}: ${direction} at ${time}ms`,
          ).toBeCloseTo(centerX, 0);
          expect(frame.status.y + frame.status.height / 2).toBeCloseTo(
            centerY,
            0,
          );
          for (const child of frame.children)
            expect(child.x + child.width / 2).toBeCloseTo(centerX, 0);
          const first = frame.children[0],
            last = frame.children.at(-1)!;
          expect((first.y + last.bottom) / 2).toBeCloseTo(centerY, 0);
          expect(first.width).toBeCloseTo(phase === 'error' ? 28 : 36, 0);
          expect(first.height).toBeCloseTo(phase === 'error' ? 28 : 36, 0);
          if (phase === 'loading') expect(frame.spinnerRunning).toBe(true);
          else expect(frame.retryClickable).toBe(true);
          if (time === 50 && width === 1000)
            await page.screenshot({
              path: `.local/player-ui/status-${phase}-${direction.toLowerCase()}.png`,
            });
        }
        await page.evaluate(() => {
          const animations = Reflect.get(
            window,
            'statusTestAnimations',
          ) as Animation[];
          for (const animation of animations) animation.finish();
          animations.length = 0;
        });
      }
    }
  }
  expect(state.errors).toEqual([]);
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

test('YouTube scrolls beneath its toolbar and Kick uses page scrolling', async ({
  page,
}) => {
  const state = await fixture(page, { app: true });
  await page.getByRole('link', { name: 'YouTube', exact: true }).click();
  await expect(page.locator('[data-page-content]')).toHaveAttribute(
    'data-page-phase',
    'idle',
  );
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
  const youtubeFeed = page.locator('[data-feed-content]');
  const feedViewport = (await youtubeFeed.boundingBox())!;
  const toolbar = page.getByRole('button', { name: 'Sort feed', exact: true });
  const toolbarTop = (await toolbar.boundingBox())!.y;
  expect((await sidebar.boundingBox())!.height).toBeLessThanOrEqual(
    viewport.height,
  );
  const card = page.locator('[data-feed-content] [data-video-id]').first();
  const before = (await card.boundingBox())!.y;
  await youtubeFeed.evaluate((element) => {
    element.scrollTop = 350;
  });
  await expect
    .poll(async () => (await card.boundingBox())!.y)
    .toBeCloseTo(before - 350, 0);
  await expect
    .poll(async () => (await sidebar.boundingBox())!.y)
    .toBeCloseTo(feedViewport.y, 0);
  expect(await workspace.evaluate((element) => element.scrollTop)).toBe(0);
  expect((await toolbar.boundingBox())!.y).toBeCloseTo(toolbarTop, 0);
  await expect(
    page.locator('[data-card-group-header][data-stuck]'),
  ).toHaveCount(1);
  await page.screenshot({ path: '.local/player-ui/youtube-scroll.png' });
  await page.getByRole('link', { name: 'Kick', exact: true }).click();
  await expect(page.locator('[data-page-content]')).toHaveAttribute(
    'data-page-phase',
    'idle',
  );
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

test('buffered HLS resumes without rebuilding its pipeline and quality changes preserve pause and position', async ({
  page,
}) => {
  const state = await fixture(page, {
    mode: 'remux',
    qualities: [
      { value: '1080p', label: '1080p' },
      { value: '720p', label: '720p' },
    ],
  });
  await decoded(page);
  await expect
    .poll(() => page.locator('video').evaluate((video) => video.currentTime))
    .toBeGreaterThan(0.3);
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  const at = await page.locator('video').evaluate((video) => video.currentTime);
  await page.getByRole('button', { name: 'Play', exact: true }).click();
  await expect
    .poll(() => page.locator('video').evaluate((video) => video.currentTime))
    .toBeGreaterThan(at + 0.2);
  expect(state.seeks).toEqual([]);
  expect(state.starts).toHaveLength(1);
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  const before = await page
    .locator('video')
    .evaluate((video) => video.currentTime);
  await page.getByRole('button', { name: 'Quality', exact: true }).click();
  await expect(page.getByRole('menu', { name: 'Quality' })).not.toContainText(
    'Mbps',
  );
  await page.getByRole('menuitemradio', { name: '720p', exact: true }).click();
  await expect.poll(() => state.starts.length).toBe(2);
  expect(state.starts[1].options.quality).toBe('720p');
  expect(Math.abs(state.starts[1].position - before)).toBeLessThan(0.2);
  await expect(
    page.getByRole('button', { name: 'Play', exact: true }),
  ).toBeEnabled();
  expect(await page.locator('video').evaluate((video) => video.paused)).toBe(
    true,
  );
  await page.getByRole('button', { name: 'Play', exact: true }).click();
  await expect
    .poll(() => page.locator('video').evaluate((video) => video.currentTime))
    .toBeGreaterThan(before + 0.2);
  expect(state.seeks).toEqual([]);
  expect(state.errors).toEqual([]);
});

test('An online stream that stops supplying media offers recovery', async ({
  page,
}, testInfo) => {
  test.setTimeout(65000);
  await fixture(page, { mode: 'remux', stalled: true });
  await decoded(page);
  await expect(
    page.getByRole('button', { name: 'Retry playback' }),
  ).toBeVisible({ timeout: 50000 });
  await expect(page.getByRole('alert')).toContainText(
    'stopped supplying media',
  );
  await testInfo.attach('stalled-stream', {
    body: await page.screenshot({
      path: '.local/player-ui/stalled-stream.png',
    }),
    contentType: 'image/png',
  });
});

test('Original playback shows its decoded resolution', async ({
  page,
}, testInfo) => {
  await fixture(page, { local: true });
  await decoded(page);
  const quality = page.getByRole('button', { name: 'Quality', exact: true });
  await expect(page.locator('.playback-mode')).toHaveText('540p · Original');
  await quality.click();
  await expect(
    page.getByRole('menuitemradio', { name: 'Auto (540p)', exact: true }),
  ).toHaveAttribute('aria-checked', 'true');
  await page
    .getByRole('menuitemradio', { name: 'Original (540p)', exact: true })
    .click();
  await expect(page.locator('.playback-mode')).toHaveText('540p · Original');
  await expect(quality).toBeEnabled();
  await quality.click();
  await expect(
    page.getByRole('menuitemradio', { name: 'Auto', exact: true }),
  ).toHaveAttribute('aria-checked', 'false');
  await testInfo.attach('original-resolution', {
    body: await page.screenshot({
      path: '.local/player-ui/original-resolution.png',
    }),
    contentType: 'image/png',
  });
});

test('Auto shows the delivered resolution only when selected', async ({
  page,
}, testInfo) => {
  const state = await fixture(page, {
    live: true,
    mode: 'remux',
    autoQuality: '1080p60',
    qualities: [
      { value: '1080p60', label: '1080p60' },
      { value: '720p60', label: '720p60' },
      { value: '480p', label: '480p' },
    ],
  });
  await decoded(page);
  const quality = page.getByRole('button', { name: 'Quality', exact: true });
  await expect(page.locator('.playback-mode')).toHaveText('1080p60 · Remuxed');
  await quality.click();
  await expect(
    page.getByRole('menuitemradio', { name: 'Auto (1080p60)', exact: true }),
  ).toHaveAttribute('aria-checked', 'true');
  await page
    .getByRole('menuitemradio', { name: '720p60', exact: true })
    .click();
  await expect(page.locator('.playback-mode')).toHaveText('720p60 · Remuxed');
  await expect(quality).toBeEnabled();
  await quality.click();
  await expect(
    page.getByRole('menuitemradio', { name: 'Auto', exact: true }),
  ).toHaveAttribute('aria-checked', 'false');
  await expect(
    page.getByRole('menuitemradio', { name: '720p60', exact: true }),
  ).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByRole('menu', { name: 'Quality' })).not.toContainText(
    'Auto (',
  );
  await testInfo.attach('manual-quality-menu', {
    body: await page.screenshot({
      path: '.local/player-ui/manual-quality-menu.png',
    }),
    contentType: 'image/png',
  });
  state.autoQuality = '480p';
  await page.getByRole('menuitemradio', { name: 'Auto', exact: true }).click();
  await expect(page.locator('.playback-mode')).toHaveText('480p · Remuxed');
  await expect(quality).toBeEnabled();
  await expect(quality).toHaveAttribute('title', 'Quality: Auto (480p)');
  await quality.click();
  await expect(
    page.getByRole('menuitemradio', { name: 'Auto (480p)', exact: true }),
  ).toHaveAttribute('aria-checked', 'true');
  await testInfo.attach('automatic-quality-menu', {
    body: await page.screenshot({
      path: '.local/player-ui/automatic-quality-menu.png',
    }),
    contentType: 'image/png',
  });
  expect(state.starts.map((start) => start.options.quality)).toEqual([
    'auto',
    '720p60',
    'auto',
  ]);
  expect(state.errors).toEqual([]);
});
