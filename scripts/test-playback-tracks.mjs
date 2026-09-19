import { request } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const origin = 'https://localhost:20443';
const api = await request.newContext({
  baseURL: origin,
  ignoreHTTPSErrors: true,
  extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
});
async function call(path, method = 'GET', data) {
  const r = await api.fetch(`/api/v1${path}`, { method, data });
  const body = await r.json();
  if (!r.ok()) throw new Error(`${path}: ${r.status()} ${body.error?.message}`);
  return body;
}
let preferences, session;
try {
  await call('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const root = (await call('/catalog/roots')).items.find(
    (r) => r.kind === 'movies',
  );
  const job = (await call(`/catalog/roots/${root.id}/scan`, 'POST')).job_id;
  for (let i = 0; i < 120; i++) {
    const result = (await call('/admin/jobs')).items.find((j) => j.id === job);
    if (result?.state === 'complete') break;
    if (result?.state === 'failed') throw new Error(result.error);
    await new Promise((r) => setTimeout(r, 500));
  }
  const item = (await call('/catalog?kind=movie')).items.find(
    (i) => i.title === 'Tracks',
  );
  assert.ok(item);
  const info = await call(`/catalog/${item.id}/playback`);
  assert.equal(
    info.sources[0].tracks.filter((t) => t.kind === 'audio').length,
    2,
  );
  preferences = await call('/playback/preferences');
  await call('/playback/preferences', 'PUT', {
    ...preferences,
    audio_language: 'fra',
    subtitle_language: 'eng',
    subtitles: true,
  });
  const options = {
    quality: 'auto',
    audio: null,
    subtitle: null,
    capabilities: {
      containers: ['mp4'],
      video: ['h264'],
      audio: ['aac'],
      hls: true,
    },
  };
  session = await call('/playback', 'POST', {
    media_id: item.id,
    options,
    position: 0,
  });
  assert.equal(session.options.audio, 2);
  assert.equal(session.mode, 'remux');
  assert.equal(session.selected_subtitle, 'embedded-3');
  const sub = await api.get(session.subtitles[0].url);
  assert.equal(sub.status(), 200);
  assert.match(await sub.text(), /WEBVTT[\s\S]*Thelxinoe subtitle fixture/);
  const playlist = await (await api.get(session.url)).text();
  const segment = playlist.split('\n').find((s) => s.startsWith('segment-'));
  const data = await (
    await api.get(new URL(segment, new URL(session.url, origin)).toString())
  ).body();
  writeFileSync('.local/selected-audio.ts', data);
  const pcm = execFileSync(
    'ffmpeg',
    [
      '-hide_banner',
      '-loglevel',
      'error',
      '-i',
      '.local/selected-audio.ts',
      '-vn',
      '-t',
      '1',
      '-ac',
      '1',
      '-ar',
      '48000',
      '-f',
      'f32le',
      'pipe:1',
    ],
    { windowsHide: true, maxBuffer: 1024 * 1024 },
  );
  let crossings = 0;
  for (let i = 4; i < pcm.length; i += 4)
    if (pcm.readFloatLE(i - 4) <= 0 && pcm.readFloatLE(i) > 0) crossings++;
  assert.ok(
    Math.abs(crossings - 880) < 8,
    `Expected selected 880 Hz audio, got ${crossings} crossings`,
  );
  const invalid = await api.post('/api/v1/playback', {
    data: {
      media_id: item.id,
      options: { ...options, audio: 999 },
      position: 0,
    },
  });
  assert.equal(invalid.status(), 400);
  console.log(
    'Saved audio/subtitle languages, embedded WebVTT and actual selected audio (880 Hz) passed.',
  );
} finally {
  if (session) await call(`/playback/${session.id}`, 'DELETE').catch(() => {});
  if (preferences) await call('/playback/preferences', 'PUT', preferences);
  await api.dispose();
}
