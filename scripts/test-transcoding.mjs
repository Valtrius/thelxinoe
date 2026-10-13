import { request, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { launchBrowser } from './ci-browser.mjs';

const origin = process.env.THELXINOE_PLAYBACK_URL ?? 'https://localhost:21443';
const api = await request.newContext({
  baseURL: origin,
  ignoreHTTPSErrors: true,
  extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
});
async function call(path, method = 'GET', data, client = api) {
  const response = await client.fetch(`/api/v1${path}`, { method, data });
  const body = await response.json();
  assert.ok(response.ok(), `${method} ${path}: ${JSON.stringify(body)}`);
  return body;
}
const capabilities = {
  containers: ['mp4'],
  video: ['h264'],
  audio: ['aac'],
  hls: true,
  allow_conversion: true,
};
let browser, viewer, original;
const sessions = [];
try {
  await call('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  original = await call('/admin/settings');
  assert.equal(original.video_transcoding, true);
  assert.equal(original.audio_transcoding, true);
  const items = (await call('/catalog?kind=movie')).items;
  const item = (title) => {
    const found = items.find((i) => i.title === title);
    assert.ok(found, `Generate playback fixtures first: ${title}`);
    return found;
  };
  const input = (title, quality = 'auto', caps = capabilities) => ({
    media_id: item(title).id,
    position: 0,
    options: { quality, audio: null, subtitle: 'off', capabilities: caps },
  });
  async function play(title, quality, caps) {
    const session = await call(
      '/playback',
      'POST',
      input(title, quality, caps),
    );
    sessions.push(session);
    return session;
  }
  async function denied(title, track, quality, caps) {
    const response = await api.post('/api/v1/playback', {
      data: input(title, quality, caps),
    });
    assert.equal(response.status(), 403);
    assert.match(
      (await response.json()).error.message,
      new RegExp(`${track} transcoding is disabled`, 'i'),
    );
  }
  async function segment(session) {
    const response = await api.get(session.url);
    assert.equal(response.status(), 200);
    const name = (await response.text())
      .split('\n')
      .find((line) => line.startsWith('segment-'));
    assert.ok(name);
    const delivered = await api.get(
      new URL(name, new URL(session.url, origin)).toString(),
    );
    assert.equal(delivered.status(), 200);
    assert.ok((await delivered.body()).length > 1000);
  }
  const me = (await call('/auth/me')).user;
  browser = await launchBrowser();
  const context = await browser.newContext({
    ignoreHTTPSErrors: true,
    storageState: await api.storageState(),
  });
  const errors = [];
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript((id) => {
    localStorage.setItem(
      `thelxinoe::${id}:navigation`,
      JSON.stringify({ section: 'Settings', settingsSection: 'server' }),
    );
  }, me.id);
  await page.goto(origin);
  const form = page.getByRole('form', { name: 'Server transcoding settings' });
  const video = form.getByRole('switch', { name: 'Allow video transcoding' });
  const audio = form.getByRole('switch', { name: 'Allow audio transcoding' });
  await expect(video).toBeChecked();
  await expect(audio).toBeChecked();
  const activeVideo = await play('Transcode');
  await segment(activeVideo);
  await video.uncheck();
  await expect
    .poll(async () => (await call('/admin/settings')).video_transcoding)
    .toBe(false);
  assert.ok([401, 404].includes((await api.get(activeVideo.url)).status()));
  await denied('Transcode', 'video');
  await denied('Direct', 'video', '144p');
  const activeAudio = await play('Audio Only');
  assert.deepEqual(activeAudio.encoding, { video: false, audio: true });
  await segment(activeAudio);
  const copied = await play('Remux');
  assert.equal(copied.mode, 'remux');
  await audio.uncheck();
  await expect
    .poll(async () => (await call('/admin/settings')).audio_transcoding)
    .toBe(false);
  assert.ok([401, 404].includes((await api.get(activeAudio.url)).status()));
  const seek = await api.post(`/api/v1/playback/${activeAudio.id}/seek`, {
    data: { position: 12 },
  });
  assert.ok([401, 404].includes(seek.status()));
  await denied('Audio Only', 'audio');
  await segment(copied);
  const direct = await play('Direct', '720p');
  assert.equal(direct.mode, 'direct');
  assert.equal(
    (await api.get(direct.url, { headers: { Range: 'bytes=0-63' } })).status(),
    206,
  );
  await page.reload();
  await expect(video).not.toBeChecked();
  await expect(audio).not.toBeChecked();
  await page.screenshot({
    path: '.local/server-transcoding-disabled.png',
    fullPage: true,
  });
  await call('/admin/settings', 'PUT', {
    timezone: original.timezone,
    time_format: original.time_format,
  });
  const persisted = await call('/admin/settings');
  assert.equal(persisted.video_transcoding, false);
  assert.equal(persisted.audio_transcoding, false);
  await video.check();
  await expect
    .poll(async () => (await call('/admin/settings')).video_transcoding)
    .toBe(true);
  assert.equal((await call('/admin/settings')).audio_transcoding, false);
  const videoOnly = await play('Transcode');
  assert.deepEqual(videoOnly.encoding, { video: true, audio: false });
  await segment(videoOnly);
  await denied('Audio Only', 'audio');
  await denied('Transcode', 'audio', 'auto', { ...capabilities, audio: [] });
  const username = 'transcoding-viewer';
  const password = 'transcoding test passphrase';
  if (!(await call('/users')).items.some((user) => user.username === username))
    await call('/users', 'POST', { username, password, role: 'user' });
  viewer = await request.newContext({
    baseURL: origin,
    ignoreHTTPSErrors: true,
    extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
  });
  await call('/auth/login', 'POST', { username, password }, viewer);
  assert.equal(
    (
      await viewer.put('/api/v1/admin/settings', {
        data: { audio_transcoding: true },
      })
    ).status(),
    403,
  );
  assert.equal((await call('/admin/settings')).audio_transcoding, false);
  assert.deepEqual(errors, []);
  writeFileSync(
    '.local/transcoding-result.json',
    JSON.stringify(
      {
        verified_at: new Date().toISOString(),
        separate_video_audio_controls: true,
        client_cannot_override_server: true,
        direct_and_remux_allowed: true,
        local_downscale_requires_video_permission: true,
        stops_only_disallowed_active_sessions: true,
        stopped_session_cannot_seek: true,
        reload_persists_policy: true,
        display_settings_preserve_policy: true,
        admin_only: true,
      },
      null,
      2,
    ),
  );
  console.log(
    'Server transcoding controls, playback enforcement, selective session shutdown and persistence passed.',
  );
} finally {
  if (original)
    await call('/admin/settings', 'PUT', {
      video_transcoding: original.video_transcoding,
      audio_transcoding: original.audio_transcoding,
    });
  for (const session of sessions)
    await api.delete(`/api/v1/playback/${session.id}`);
  await browser?.close();
  await viewer?.dispose();
  await api.dispose();
}
