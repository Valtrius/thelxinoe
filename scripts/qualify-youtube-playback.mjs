// Opt-in qualification against an existing development account and public media.
import { request } from '@playwright/test';
import assert from 'node:assert/strict';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { execFileSync } from 'node:child_process';

const origin = process.env.THELXINOE_YOUTUBE_URL;
const username = process.env.THELXINOE_YOUTUBE_USER;
const password = process.env.THELXINOE_YOUTUBE_PASSWORD;
const ids = process.argv.slice(2);
if (
  !origin ||
  !username ||
  !password ||
  !ids.length ||
  ids.some((id) => !/^[\w-]{11}$/.test(id))
)
  throw new Error(
    'Set THELXINOE_YOUTUBE_URL, THELXINOE_YOUTUBE_USER and THELXINOE_YOUTUBE_PASSWORD, and supply existing YouTube video IDs',
  );
const output =
  process.env.THELXINOE_YOUTUBE_OUTPUT ?? '.local/youtube-playback-live.json';
mkdirSync(dirname(output), { recursive: true });
const api = await request.newContext({
  baseURL: origin,
  ignoreHTTPSErrors: true,
  extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
});
const evidence = [];
let loggedIn = false;
async function call(path, method = 'GET', data) {
  const response = await api.fetch(`/api/v1${path}`, {
    method,
    data,
    timeout: 150000,
  });
  const body = await response.json();
  assert.equal(
    response.ok(),
    true,
    `${method} ${path}: HTTP ${response.status()}, ${body.error?.message}`,
  );
  return body;
}
try {
  await call('/auth/login', 'POST', { username, password });
  loggedIn = true;
  const before = await call('/admin/online');
  for (const id of ids) {
    let session;
    try {
      const info = await call(`/catalog/youtube:${id}/playback`);
      assert.ok(
        info.sources[0].probe.streams.some(
          (stream) => stream.codec_type === 'video',
        ),
      );
      const started = performance.now();
      session = await call('/playback', 'POST', {
        media_id: `youtube:${id}`,
        position: 0,
        options: {
          quality: 'auto',
          audio: null,
          subtitle: 'off',
          capabilities: {
            containers: [],
            video: ['h264'],
            audio: ['aac'],
            hls: true,
          },
        },
      });
      const prepared = Math.round(performance.now() - started);
      // Browsers receive the provider's original files as byte-range HLS.
      assert.equal(session.mode, 'direct');
      assert.deepEqual(session.encoding, { video: false, audio: false });
      const fetch = async (url, range) => {
        const response = await api.get(url.toString(), {
          headers: range ? { Range: range } : {},
        });
        assert.equal(response.status(), range ? 206 : 200);
        return response.body();
      };
      const master = new URL(session.url, origin);
      const variants = (await fetch(master)).toString();
      const target = Math.min(75, session.duration);
      const tracks = {};
      let firstMedia;
      for (const [track, uri] of [
        ['video', variants.split('\n').find((l) => l && !l.startsWith('#'))],
        ['audio', /URI="([^"]+)"/.exec(variants)[1]],
      ]) {
        const address = new URL(uri, master);
        const playlist = (await fetch(address)).toString();
        assert.ok(playlist.includes('#EXT-X-ENDLIST'));
        const bytes = [];
        let seconds = 0,
          duration = 0,
          range;
        for (const line of playlist.split('\n')) {
          const init = /^#EXT-X-MAP:URI="([^"]+)"/.exec(line);
          if (init) bytes.push(await fetch(new URL(init[1], address)));
          else if (line.startsWith('#EXTINF:'))
            duration = Number(line.slice(8).split(',')[0]);
          else if (line.startsWith('#EXT-X-BYTERANGE:')) {
            const [size, offset] = line.slice(17).split('@').map(Number);
            range = `bytes=${offset}-${offset + size - 1}`;
          } else if (line && !line.startsWith('#')) {
            if (seconds >= target) break;
            bytes.push(await fetch(new URL(line, address), range));
            firstMedia ??= Math.round(performance.now() - started);
            seconds += duration;
          }
        }
        assert.ok(
          seconds >= target - 0.1,
          `Only ${seconds} seconds of ${target} were delivered`,
        );
        const delivered = `${dirname(output)}/youtube-${id}-${track}.mp4`;
        writeFileSync(delivered, Buffer.concat(bytes));
        const probe = JSON.parse(
          execFileSync(
            'ffprobe',
            [
              '-v',
              'error',
              '-show_entries',
              'stream=codec_name,codec_type,width,height',
              '-of',
              'json',
              delivered,
            ],
            { encoding: 'utf8', windowsHide: true },
          ),
        );
        execFileSync(
          'ffmpeg',
          ['-v', 'error', '-i', delivered, '-f', 'null', '-'],
          { windowsHide: true },
        );
        tracks[track] = { codecs: probe.streams, delivered_seconds: seconds };
      }
      assert.equal(tracks.video.codecs[0].codec_name, 'h264');
      assert.equal(tracks.audio.codecs[0].codec_name, 'aac');
      evidence.push({
        video: id,
        mode: session.mode,
        quality: session.quality,
        encoding: session.encoding,
        prepare_ms: prepared,
        first_media_ms: firstMedia,
        tracks,
      });
    } finally {
      if (session) await api.delete(`/api/v1/playback/${session.id}`);
    }
  }
  const after = await call('/admin/online');
  writeFileSync(
    output,
    JSON.stringify(
      {
        verified_at: new Date().toISOString(),
        passed: true,
        network_mode: after.youtube_network,
        initial_family: before.youtube_network_status.current,
        successful_family: after.youtube_network_status.current,
        videos: evidence,
      },
      null,
      2,
    ),
  );
  console.log(
    `YouTube codec inspection and decoded original playback passed for ${ids.length} videos.`,
  );
} catch (error) {
  writeFileSync(
    output,
    JSON.stringify(
      {
        verified_at: new Date().toISOString(),
        passed: false,
        error: error.message,
        videos: evidence,
      },
      null,
      2,
    ),
  );
  throw error;
} finally {
  try {
    if (loggedIn) await api.post('/api/v1/auth/logout');
  } finally {
    await api.dispose();
  }
}
