import { request } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const origin = process.env.THELXINOE_PLAYBACK_URL ?? 'https://localhost:20443';
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
      // Skip AAC priming samples: their near-silent ringing adds zero crossings
      // that do not represent the selected track's steady tone.
      '-ss',
      '0.25',
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
  assert.equal(
    pcm.length,
    48000 * 4,
    'Expected one full second of float32 audio',
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
  await call(`/playback/${session.id}`, 'DELETE');

  await call('/playback/preferences', 'PUT', {
    ...preferences,
    subtitles: false,
    subtitle_size: 1.5,
  });
  assert.equal((await call('/playback/preferences')).subtitle_size, 1.5);
  const oversized = await api.put('/api/v1/playback/preferences', {
    data: { ...preferences, subtitle_size: 3 },
  });
  assert.equal(oversized.status(), 400);
  const movies = (await call('/catalog?kind=movie')).items;
  const styled = movies.find((i) => i.title === 'Styled');
  const picture = movies.find((i) => i.title === 'Picture');
  assert.ok(styled && picture);
  // The web player has picture subtitles burned in; desktop and Jellyfin
  // clients select embedded tracks, forced ones included, themselves.
  const web = { ...options.capabilities, burn_subtitles: true };
  const native = { ...options.capabilities, native_tracks: true };
  const play = (media, capabilities, subtitle = null, position = 0) =>
    call('/playback', 'POST', {
      media_id: media.id,
      options: { ...options, subtitle, capabilities },
      position,
    });

  // A forced styled track in the audio's language shows by default, in its
  // own format and with the font the file attaches.
  session = await play(styled, web);
  assert.equal(session.selected_subtitle, 'embedded-2');
  const track = session.subtitles.find((s) => s.id === 'embedded-2');
  assert.equal(track.codec, 'ass');
  const script = await (await api.get(`${track.url}&format=ass`)).text();
  assert.match(script, /Style: Sign,Liberation Sans,24,/);
  assert.match(script, /\{\\pos\(160,40\)\}Styled fixture/);
  assert.match(await (await api.get(track.url)).text(), /^WEBVTT/);
  assert.equal(session.fonts.length, 1);
  const font = await api.get(session.fonts[0]);
  assert.equal(font.status(), 200);
  assert.ok(
    (await font.body()).equals(
      readFileSync('frontend/node_modules/jassub/dist/default.woff2'),
    ),
  );
  await call(`/playback/${session.id}`, 'DELETE');
  session = await play(styled, native);
  assert.equal(session.selected_subtitle, null);
  await call(`/playback/${session.id}`, 'DELETE');

  // The white box of the picture tracks covers the bottom of the black
  // picture from 4 s to 14 s.
  const burned = async (url, at = 0) => {
    const playlist = await (await api.get(url)).text();
    const name = playlist.split('\n').find((s) => s.startsWith('segment-'));
    writeFileSync(
      '.local/burned-subtitle.ts',
      await (
        await api.get(new URL(name, new URL(url, origin)).toString())
      ).body(),
    );
    // Decoded from the segment's start: a seek into it can miss its references.
    const gray = execFileSync(
      'ffmpeg',
      [
        ...['-hide_banner', '-loglevel', 'error', '-i'],
        ...['.local/burned-subtitle.ts', '-ss', String(at), '-frames:v', '1'],
        ...['-vf', 'scale=320:180', '-pix_fmt', 'gray', '-f', 'rawvideo'],
        'pipe:1',
      ],
      { windowsHide: true },
    );
    assert.equal(gray.length, 320 * 180);
    assert.ok(gray[40 * 320 + 160] < 40, 'The picture above the box is black');
    return gray[140 * 320 + 160] > 200;
  };
  // A forced picture track never shows by default: it needs a conversion, so
  // a file that plays directly still does.
  session = await play(picture, { ...web, containers: ['mkv'] });
  assert.equal(session.selected_subtitle, null);
  assert.equal(session.mode, 'direct');
  await call(`/playback/${session.id}`, 'DELETE');
  // Chosen, the DVD track is burned in and keeps its timeline across seeks.
  // A seek to 8 s reads from the keyframe at 8 s, after the box's packet, so
  // only the preroll shows the box there; it is gone after it ends.
  session = await play(picture, web, 'embedded-3');
  assert.equal(session.selected_subtitle, 'embedded-3');
  assert.equal(session.mode, 'transcode');
  assert.equal(session.encoding.subtitle, 3);
  assert.match(
    session.conversion_reason,
    /Picture subtitles are burned into the video/,
  );
  assert.equal(session.subtitles.find((s) => s.id === 'embedded-3').url, null);
  assert.ok(await burned(session.url, 5), 'The DVD subtitle is burned in');
  for (const [position, shown] of [
    [8, true],
    [15, false],
  ]) {
    const seek = await call(`/playback/${session.id}/seek`, 'POST', {
      position,
    });
    assert.equal(seek.timeline_start, position);
    assert.equal(
      await burned(seek.url),
      shown,
      `The subtitle shows ${shown ? 'after' : 'only before'} ${position} s`,
    );
  }
  await call(`/playback/${session.id}`, 'DELETE');
  session = await play(picture, web, 'embedded-2');
  assert.ok(await burned(session.url, 5), 'The PGS subtitle is burned in');
  await call(`/playback/${session.id}`, 'DELETE');
  // A client that can't burn them in is never given picture subtitles.
  session = await play(picture, options.capabilities);
  assert.equal(session.selected_subtitle, null);
  assert.equal(session.mode, 'remux');
  assert.deepEqual(session.subtitles, []);
  const refused = await api.post('/api/v1/playback', {
    data: {
      media_id: picture.id,
      options: { ...options, subtitle: 'embedded-2' },
      position: 0,
    },
  });
  assert.equal(refused.status(), 400);
  console.log(
    'Saved audio/subtitle languages, embedded WebVTT, actual selected audio (880 Hz), forced and styled text tracks with their fonts, and chosen picture subtitles burned in passed.',
  );
} finally {
  if (session) await call(`/playback/${session.id}`, 'DELETE').catch(() => {});
  if (preferences) await call('/playback/preferences', 'PUT', preferences);
  await api.dispose();
}
