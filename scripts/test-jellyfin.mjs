import { request } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const api = await request.newContext({
  baseURL: 'https://localhost:21443',
  ignoreHTTPSErrors: true,
  extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
});
async function call(path, method = 'GET', data) {
  const r = await api.fetch(`/api/v1${path}`, { method, data });
  if (!r.ok()) throw new Error(`${method} ${path.split('?')[0]} ${r.status()}`);
  return r.json();
}
const credentials = {
  username: 'admin',
  password: 'test-only long passphrase',
};
if ((await call('/setup')).setup_required) {
  const code = execFileSync(
    'docker',
    [
      'compose',
      '-p',
      'thelxinoe-compat',
      '-f',
      'compose.test.yaml',
      'exec',
      '-T',
      'server',
      'cat',
      '/var/lib/thelxinoe/secrets/setup-token',
    ],
    { encoding: 'utf8', windowsHide: true },
  ).trim();
  await call('/setup', 'POST', { ...credentials, setup_token: code });
} else await call('/auth/login', 'POST', credentials);
const roots = (await call('/catalog/roots')).items;
for (const kind of ['movies', 'shows', 'music']) {
  const root =
    roots.find((r) => r.kind === kind) ??
    (await call('/catalog/roots', 'POST', {
      kind,
      name: `Compatibility ${kind}`,
      path: `/data/playback/${kind}`,
    }));
  const job = (await call(`/catalog/roots/${root.id}/scan`, 'POST')).job_id;
  for (let i = 0; i < 120; i++) {
    const status = (await call('/admin/jobs')).items.find((j) => j.id === job);
    if (status?.state === 'complete') break;
    if (status?.state === 'failed') throw new Error(status.error);
    await new Promise((r) => setTimeout(r, 500));
  }
}
const compat = await request.newContext({
  baseURL: 'http://127.0.0.1:18787',
  extraHTTPHeaders: {
    Authorization:
      'MediaBrowser Client="Protocol validation", Device="Automated test", DeviceId="thelxinoe-compat-test", Version="1"',
  },
});
let r = await compat.post('/Users/AuthenticateByName', {
  data: { Username: credentials.username, Pw: credentials.password },
});
assert.equal(r.status(), 200);
const login = await r.json();
const headers = { 'X-Emby-Token': login.AccessToken };
r = await compat.get('/UserViews', { headers });
assert.equal(r.status(), 200);
const views = await r.json();
assert.ok(views.Items.length >= 3);
r = await compat.get('/Items?IncludeItemTypes=Movie&Recursive=true', {
  headers,
});
assert.equal(r.status(), 200);
const movies = await r.json();
assert.ok(movies.Items.some((i) => i.Name === 'Direct'));
r = await compat.get('/Items?SearchTerm=Direct&IncludeItemTypes=Movie', {
  headers,
});
assert.equal((await r.json()).TotalRecordCount, 1);
r = await compat.get('/api/v1/auth/me', {
  headers: { Authorization: `Bearer ${login.AccessToken}` },
});
assert.equal(r.status(), 401);
const direct = movies.Items.find((i) => i.Name === 'Direct');
await call(`/catalog/${direct.Id}/state`, 'PUT', { watched: false });
const profile = {
  DirectPlayProfiles: [
    { Type: 'Video', Container: 'mp4', VideoCodec: 'h264', AudioCodec: 'aac' },
  ],
  TranscodingProfiles: [
    {
      Type: 'Video',
      Container: 'ts',
      Protocol: 'hls',
      VideoCodec: 'h264',
      AudioCodec: 'aac',
    },
  ],
};
r = await compat.post(`/Items/${direct.Id}/PlaybackInfo`, {
  headers,
  data: {
    DeviceProfile: profile,
    EnableDirectPlay: true,
    EnableDirectStream: true,
    EnableTranscoding: true,
    SubtitleStreamIndex: -1,
  },
});
assert.equal(r.status(), 200);
const playback = await r.json();
const source = playback.MediaSources[0];
assert.equal(source.SupportsDirectPlay, true);
const streamPath = `/Videos/${direct.Id}/stream?Static=true&MediaSourceId=${source.Id}&PlaySessionId=${playback.PlaySessionId}&tag=${source.ETag}`;
r = await compat.get(streamPath, { headers: { Range: 'bytes=0-63' } });
assert.equal(r.status(), 206);
assert.equal((await r.body()).length, 64);
r = await compat.get(
  `/Videos/${direct.Id}/stream?PlaySessionId=${playback.PlaySessionId}`,
);
assert.equal(r.status(), 401);
r = await compat.post('/Sessions/Playing/Progress', {
  headers,
  data: {
    ItemId: direct.Id,
    PlaySessionId: playback.PlaySessionId,
    PositionTicks: 50_000_000,
    IsPaused: true,
  },
});
assert.equal(r.status(), 204);
r = await compat.get(`/Items/${direct.Id}`, { headers });
assert.equal((await r.json()).UserData.PlaybackPositionTicks, 50_000_000);
r = await compat.post('/Sessions/Playing/Stopped', {
  headers,
  data: {
    ItemId: direct.Id,
    PlaySessionId: playback.PlaySessionId,
    PositionTicks: 170_000_000,
  },
});
assert.equal(r.status(), 204);
r = await compat.get(`/Items/${direct.Id}`, { headers });
assert.equal((await r.json()).UserData.Played, true);
r = await compat.get(streamPath);
assert.equal(r.status(), 401);
await call(`/catalog/${direct.Id}/state`, 'PUT', { watched: false });
const serverLogs = execFileSync(
  'docker',
  [
    'compose',
    '-p',
    'thelxinoe-compat',
    '-f',
    'compose.test.yaml',
    'logs',
    '--no-color',
    'server',
  ],
  { encoding: 'utf8', windowsHide: true },
);
assert.ok(!serverLogs.includes(login.AccessToken));
assert.ok(!serverLogs.includes(source.ETag));
writeFileSync(
  '.local/jellyfin-result.json',
  JSON.stringify(
    {
      server: login.ServerId,
      views: views.Items.map((i) => i.Name),
      movies: movies.Items.map((i) => i.Name),
      compatibilityTokenRejectedByFirstParty: true,
      grantedDirectPlaybackAndProgress: true,
      revokedStreamRejected: true,
      queryCredentialsAbsentFromLogs: true,
    },
    null,
    2,
  ),
);
console.log(
  'Jellyfin authentication, library browsing, search, and transport separation passed.',
);
await compat.dispose();
await api.dispose();
