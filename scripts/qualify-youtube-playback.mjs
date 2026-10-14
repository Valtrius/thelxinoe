// Opt-in qualification against an existing development account and public media.
import { request } from '@playwright/test';
import assert from 'node:assert/strict';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { execFileSync } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

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
      assert.equal(session.mode, 'remux');
      assert.deepEqual(session.encoding, { video: false, audio: false });
      const bytes = [];
      const seen = new Set();
      const target = Math.min(75, session.duration);
      let seconds = 0,
        initLoaded = false,
        lastSegment = Date.now();
      while (seconds < target) {
        const playlist = await api.get(session.url);
        assert.equal(playlist.status(), 200);
        const manifest = await playlist.text();
        const init = manifest.match(/#EXT-X-MAP:URI="([^"]+)"/);
        if (init && !initLoaded) {
          const response = await api.get(
            new URL(init[1], new URL(session.url, origin)).toString(),
          );
          assert.equal(response.status(), 200);
          bytes.push(await response.body());
          initLoaded = true;
        }
        let duration = 0;
        for (const line of manifest.split('\n')) {
          if (line.startsWith('#EXTINF:'))
            duration = Number(line.slice(8).split(',')[0]);
          if (!line.startsWith('segment-') || seen.has(line)) continue;
          const response = await api.get(
            new URL(line, new URL(session.url, origin)).toString(),
          );
          assert.equal(response.status(), 200);
          bytes.push(await response.body());
          seen.add(line);
          seconds += duration;
          lastSegment = Date.now();
        }
        if (manifest.includes('#EXT-X-ENDLIST')) break;
        assert.ok(
          Date.now() - lastSegment < 25000,
          'The public stream stopped producing segments',
        );
        if (seconds < target) await delay(1000);
      }
      assert.ok(
        seconds >= target - 0.1,
        `Only ${seconds} seconds of ${target} were delivered`,
      );
      const delivered = `${dirname(output)}/youtube-${id}.mp4`;
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
      assert.ok(probe.streams.some((stream) => stream.codec_name === 'h264'));
      assert.ok(probe.streams.some((stream) => stream.codec_name === 'aac'));
      execFileSync(
        'ffmpeg',
        ['-v', 'error', '-i', delivered, '-f', 'null', '-'],
        { windowsHide: true },
      );
      evidence.push({
        video: id,
        mode: session.mode,
        quality: session.quality,
        encoding: session.encoding,
        codecs: probe.streams,
        segments: seen.size,
        delivered_seconds: seconds,
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
