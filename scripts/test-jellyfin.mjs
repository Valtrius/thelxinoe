import { request } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import dgram from 'node:dgram';
const discovery = dgram.createSocket('udp4');
try {
  const discovered = await new Promise((resolve, reject) => {
    const timeout = setTimeout(
      () => reject(new Error('LAN discovery timed out')),
      3000,
    );
    discovery.once('message', (data) => {
      clearTimeout(timeout);
      resolve(JSON.parse(data));
    });
    discovery.send('who is JellyfinServer?', 18787, '127.0.0.1');
  });
  assert.equal(discovered.Address, 'http://127.0.0.1:18787');
  assert.ok(discovered.Id);
  assert.match(discovered.Name, /^Thelxinoe /);
} finally {
  discovery.close();
}
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
  await call('/setup', 'POST', credentials);
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
const otherLogin = await compat.post('/Users/AuthenticateByName', {
  headers: {
    Authorization:
      'MediaBrowser Client="Protocol validation", Device="Second test", DeviceId="thelxinoe-compat-other", Version="1"',
  },
  data: { Username: credentials.username, Pw: credentials.password },
});
assert.equal(otherLogin.status(), 200);
const otherHeaders = { 'X-Emby-Token': (await otherLogin.json()).AccessToken };
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
r = await compat.get('/Items?IncludeItemTypes=Audio', { headers });
const tracks = (await r.json()).Items;
assert.ok(tracks.length >= 2);
assert.ok(tracks.every((t) => t.IndexNumber < 10000));
const playlists = (await call('/playlists')).items;
if (!playlists.some((p) => p.name === 'TV validation mix')) {
  await call('/playlists', 'POST', {
    name: 'TV validation mix',
    items: [tracks[0].Id, tracks[1].Id, tracks[0].Id],
  });
}
const musicPath = `/Audio/${tracks[0].Id}/universal?Container=mp3&Container=flac`;
r = await compat.get(musicPath, {
  headers: { ...headers, Range: 'bytes=0-63' },
});
assert.equal(r.status(), 206);
assert.equal((await r.body()).subarray(0, 4).toString(), 'fLaC');
assert.equal((await compat.get(musicPath)).status(), 401);
assert.equal(
  (
    await compat.post('/Sessions/Playing/Progress', {
      headers: otherHeaders,
      data: { ItemId: tracks[0].Id, PositionTicks: 10_000_000 },
    })
  ).status(),
  400,
  'Another device cannot infer the current audio session',
);
for (const [suffix, position] of [
  ['', 0],
  ['/Progress', 10_000_000],
  ['/Stopped', 20_000_000],
]) {
  r = await compat.post(`/Sessions/Playing${suffix}`, {
    headers,
    data: { ItemId: tracks[0].Id, PositionTicks: position, IsPaused: false },
  });
  assert.equal(r.status(), 204, 'Implicit audio playback progress');
}
r = await compat.post('/Sessions/Playing/Progress', {
  headers,
  data: { ItemId: movies.Items[0].Id, PositionTicks: 1 },
});
assert.equal(
  r.status(),
  400,
  'Video progress still requires an explicit play session',
);
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
  SubtitleProfiles: [{ Format: 'srt', Method: 'External' }],
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
const subtitle = source.MediaStreams.find((s) => s.Type === 'Subtitle');
assert.equal(subtitle.Codec, 'srt');
for (const format of ['srt', 'vtt']) {
  const url = new URL(subtitle.DeliveryUrl, 'http://127.0.0.1:18787');
  url.searchParams.set('format', format);
  r = await compat.get(url.toString());
  assert.equal(r.status(), 200);
  const body = await r.text();
  assert.ok(body.includes('-->'));
  assert.equal(body.startsWith('WEBVTT'), format === 'vtt');
}
const streamPath = `/Videos/${direct.Id}/stream?Static=true&MediaSourceId=${source.Id}&PlaySessionId=${playback.PlaySessionId}&tag=${source.ETag}`;
r = await compat.get(streamPath, { headers: { Range: 'bytes=0-63' } });
assert.equal(r.status(), 206);
assert.equal((await r.body()).length, 64);
const tvUrl = `/Videos/${direct.Id}/stream.mp4?Static=true&MediaSourceId=${source.Id}&tag=${source.ETag}`;
assert.equal(
  (await compat.get(tvUrl, { headers: { Range: 'bytes=0-63' } })).status(),
  206,
);
assert.equal(
  (
    await compat.get(
      tvUrl.replace(source.Id, movies.Items.find((i) => i.Id !== direct.Id).Id),
    )
  ).status(),
  401,
);
assert.equal(
  (
    await compat.get(
      `${tvUrl}&PlaySessionId=00000000-0000-0000-0000-000000000000`,
    )
  ).status(),
  401,
);
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
for (const mode of [
  'remux',
  'transcode',
  ...(process.argv.includes('--tv') ? ['limited', 'hdr'] : []),
]) {
  const item = movies.Items.find(
    (i) => i.Name === (mode === 'hdr' ? 'TV HDR' : 'Remux'),
  );
  assert.ok(item, 'Generate playback and TV fixtures first');
  const deviceProfile = structuredClone(profile);
  if (mode === 'limited')
    deviceProfile.CodecProfiles = [
      {
        Type: 'Video',
        Codec: 'h264',
        Conditions: [
          {
            Property: 'Width',
            Condition: 'LessThanEqual',
            Value: '160',
            IsRequired: true,
          },
          {
            Property: 'Height',
            Condition: 'LessThanEqual',
            Value: '90',
            IsRequired: true,
          },
          {
            Property: 'VideoFramerate',
            Condition: 'LessThanEqual',
            Value: '12',
            IsRequired: true,
          },
        ],
      },
    ];
  r = await compat.post(`/Items/${item.Id}/PlaybackInfo`, {
    headers,
    data: {
      DeviceProfile: deviceProfile,
      EnableDirectPlay: false,
      EnableDirectStream: mode === 'remux',
      EnableTranscoding: true,
      SubtitleStreamIndex: -2,
      ...(mode === 'limited' ? { MaxStreamingBitrate: 2_500_000 } : {}),
    },
  });
  assert.equal(r.status(), 200, `${mode} playback info`);
  const session = await r.json();
  const converted = session.MediaSources[0];
  assert.equal(converted.SupportsDirectPlay, false);
  assert.equal(converted.SupportsTranscoding, true);
  assert.equal(converted.SupportsDirectStream, false);
  const playlistUrl = new URL(
    converted.TranscodingUrl,
    'http://127.0.0.1:18787',
  );
  r = await compat.get(playlistUrl.toString());
  assert.equal(r.status(), 200);
  const playlist = await r.text();
  assert.ok(playlist.includes('#EXT-X-PLAYLIST-TYPE:VOD'));
  assert.ok(playlist.includes('#EXT-X-ENDLIST'));
  const segments = playlist.split('\n').filter((s) => s.startsWith('segment-'));
  assert.ok(segments.length >= 2);
  // A TV seek may request the end before the beginning. Every segment must
  // remain addressable and independently decodable in either direction.
  for (const index of [segments.length - 1, 0]) {
    r = await compat.get(new URL(segments[index], playlistUrl).toString());
    assert.equal(r.status(), 200, `${mode} segment ${index}`);
    const path = `.local/jellyfin-${mode}-${index}.ts`;
    writeFileSync(path, await r.body());
    const probe = JSON.parse(
      execFileSync(
        'ffprobe',
        ['-v', 'error', '-show_streams', '-of', 'json', path],
        { encoding: 'utf8', windowsHide: true },
      ),
    );
    assert.ok(probe.streams.some((s) => s.codec_name === 'h264'));
    if (mode === 'limited') {
      const video = probe.streams.find((s) => s.codec_type === 'video');
      assert.equal(video.width, 160);
      assert.equal(video.height, 90);
      assert.equal(video.r_frame_rate, '12/1');
    }
    if (mode === 'hdr') {
      const video = probe.streams.find((s) => s.codec_type === 'video');
      assert.equal(video.color_transfer, 'bt709');
      assert.equal(video.color_primaries, 'bt709');
      assert.equal(video.pix_fmt, 'yuv420p');
    }
    execFileSync(
      'ffmpeg',
      [
        '-v',
        'error',
        '-xerror',
        '-i',
        path,
        '-fps_mode',
        'passthrough',
        '-enc_time_base',
        '1:90000',
        '-f',
        'null',
        '-',
      ],
      { windowsHide: true },
    );
  }
  const cancelPath = `/Videos/ActiveEncodings?PlaySessionId=${session.PlaySessionId}`;
  assert.equal(
    (await compat.delete(cancelPath, { headers: otherHeaders })).status(),
    404,
  );
  assert.equal(
    (await compat.delete(`${cancelPath}&DeviceId=wrong`, { headers })).status(),
    404,
  );
  r =
    mode === 'remux'
      ? await compat.delete(`${cancelPath}&DeviceId=thelxinoe-compat-test`, {
          headers,
        })
      : await compat.post('/Sessions/Playing/Stopped', {
          headers,
          data: {
            ItemId: item.Id,
            PlaySessionId: session.PlaySessionId,
            PositionTicks: 0,
          },
        });
  assert.equal(r.status(), 204);
  r = await compat.get(playlistUrl.toString());
  assert.equal(r.status(), 401);
}
assert.equal(
  (
    await compat.get(musicPath, {
      headers: { ...otherHeaders, Range: 'bytes=0-63' },
    })
  ).status(),
  206,
);
assert.equal(
  (await compat.post('/Sessions/Logout', { headers: otherHeaders })).status(),
  204,
);
assert.equal(
  (await compat.get(musicPath, { headers: otherHeaders })).status(),
  401,
);
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
      seekableRemuxAndTranscode: true,
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
