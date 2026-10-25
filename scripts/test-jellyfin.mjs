import { request } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import dgram from 'node:dgram';
import { Jellyfin, API_VERSION } from '@jellyfin/sdk';
import { getAuthenticationApi } from '@jellyfin/sdk/lib/utils/api/authentication-api.js';
import { getDisplayPreferenceApi } from '@jellyfin/sdk/lib/utils/api/display-preference-api.js';
import { getFilterApi } from '@jellyfin/sdk/lib/utils/api/filter-api.js';
import { getLibraryApi } from '@jellyfin/sdk/lib/utils/api/library-api.js';
import { getSystemApi } from '@jellyfin/sdk/lib/utils/api/system-api.js';
import { getUserDataApi } from '@jellyfin/sdk/lib/utils/api/user-data-api.js';
const httpPort = Number(process.env.THELXINOE_TEST_HTTP_PORT ?? 18787);
const origin = `http://127.0.0.1:${httpPort}`;
const sdk = new Jellyfin({
  clientInfo: { name: 'Protocol validation', version: '1' },
  deviceInfo: { name: 'Automated test', id: 'thelxinoe-compat-test' },
});
const sdkApi = sdk.createApi(origin);
const publicInfo = (await getSystemApi(sdkApi).getPublicSystemInfo()).data;
assert.equal(publicInfo.Version, '12.1.0');
assert.match(publicInfo.ServerName, /^Thelxinoe /);
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
    discovery.send('who is JellyfinServer?', httpPort, '127.0.0.1');
  });
  assert.equal(discovered.Address, origin);
  assert.ok(discovered.Id);
  assert.match(discovered.Name, /^Thelxinoe /);
} finally {
  discovery.close();
}
const api = await request.newContext({
  baseURL: process.env.THELXINOE_PLAYBACK_URL ?? 'https://localhost:21443',
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
    roots.find((r) => r.path === `/media/playback/${kind}`) ??
    (await call('/catalog/roots', 'POST', {
      kind,
      name: `Compatibility ${kind}`,
      path: `/media/playback/${kind}`,
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
  baseURL: origin,
  extraHTTPHeaders: {
    Authorization:
      'MediaBrowser Client="Protocol validation", Device="Automated test", DeviceId="thelxinoe-compat-test", Version="1"',
  },
});
const login = (
  await getAuthenticationApi(sdkApi).authenticateUserByName({
    authenticateUserByName: {
      Username: credentials.username,
      Pw: credentials.password,
    },
  })
).data;
sdkApi.update({ accessToken: login.AccessToken });
assert.equal(
  (await getSystemApi(sdkApi).getSystemInfo()).data.Version,
  publicInfo.Version,
);
let r;
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
const pairingClient = await request.newContext({
  baseURL: origin,
  extraHTTPHeaders: {
    Authorization:
      'MediaBrowser Client="Protocol validation", Device="Pairing test", DeviceId="thelxinoe-compat-pairing", Version="1"',
  },
});
const pairing = await pairingClient.post('/QuickConnect/Initiate', {
  data: {},
});
assert.equal(pairing.status(), 200);
const pendingPairing = await pairing.json();
const review = await call('/auth/quick-connect/inspect', 'POST', {
  code: pendingPairing.Code,
});
await call('/auth/quick-connect/approve', 'POST', {
  code: pendingPairing.Code,
  confirmation: review.confirmation,
});
const connected = await pairingClient.post(
  '/Users/AuthenticateWithQuickConnect',
  {
    data: { Secret: pendingPairing.Secret },
  },
);
assert.equal(connected.status(), 200);
assert.equal((await connected.json()).User.Id, login.User.Id);
assert.equal(
  (
    await pairingClient.post('/Users/AuthenticateWithQuickConnect', {
      data: { Secret: pendingPairing.Secret },
    })
  ).status(),
  401,
);
await pairingClient.dispose();
r = await compat.get('/UserViews', { headers });
assert.equal(r.status(), 200);
const views = await r.json();
assert.ok(views.Items.length >= 3);
const displayPreferencesApi = getDisplayPreferenceApi(sdkApi);
const libraryPreferences = [];
for (const view of views.Items) {
  assert.equal(typeof view.DisplayPreferencesId, 'string', view.Name);
  assert.ok(view.DisplayPreferencesId.length > 0, view.Name);
  for (const path of [
    `/Items/${view.Id}`,
    `/Users/${login.User.Id}/Items/${view.Id}`,
  ]) {
    const item = await compat.get(path, { headers });
    assert.equal(item.status(), 200, path);
    assert.equal(
      (await item.json()).DisplayPreferencesId,
      view.DisplayPreferencesId,
      `${view.Name} keeps its preferences when opened`,
    );
  }
  const parameters = {
    displayPreferencesId: view.DisplayPreferencesId,
    client: 'jellyfin-androidtv',
  };
  const original = (
    await displayPreferencesApi.getDisplayPreferences(parameters)
  ).data;
  const gridDirection =
    original.CustomPrefs.GridDirection === 'VERTICAL'
      ? 'HORIZONTAL'
      : 'VERTICAL';
  await displayPreferencesApi.updateDisplayPreferences({
    ...parameters,
    displayPreferencesDto: {
      ...original,
      CustomPrefs: { ...original.CustomPrefs, GridDirection: gridDirection },
    },
  });
  const saved = (await displayPreferencesApi.getDisplayPreferences(parameters))
    .data;
  assert.equal(saved.CustomPrefs.GridDirection, gridDirection, view.Name);
  await displayPreferencesApi.updateDisplayPreferences({
    ...parameters,
    displayPreferencesDto: original,
  });
  libraryPreferences.push({
    name: view.Name,
    displayPreferencesId: view.DisplayPreferencesId,
    gridDirection,
  });
}
assert.equal(
  new Set(libraryPreferences.map((view) => view.displayPreferencesId)).size,
  libraryPreferences.length,
  'Each library has independent display preferences',
);
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
const userDataApi = getUserDataApi(sdkApi);
const itemParameters = { itemId: direct.Id, userId: login.User.Id };
const originalData = (await userDataApi.getItemUserData(itemParameters)).data;
const filters = (
  await getFilterApi(sdkApi).getQueryFilters({
    parentId: views.Items.find((i) => i.CollectionType === 'movies').Id,
    includeItemTypes: ['Movie'],
    recursive: true,
  })
).data;
assert.ok(Array.isArray(filters.Genres));
assert.ok(Array.isArray(filters.Tags));
assert.ok(filters.AudioLanguages.some((language) => language.Value === 'fra'));
assert.ok(
  filters.SubtitleLanguages.some((language) => language.Value === 'eng'),
);
const frenchMovies = await compat.get(
  '/Items?IncludeItemTypes=Movie&AudioLanguages=fra',
  { headers },
);
assert.deepEqual(
  (await frenchMovies.json()).Items.map((item) => item.Name),
  ['Tracks'],
);
const musicFilters = (
  await getFilterApi(sdkApi).getQueryFilters({
    parentId: views.Items.find((item) => item.CollectionType === 'music').Id,
    recursive: true,
  })
).data;
assert.deepEqual(musicFilters.SubtitleLanguages, []);
const collections = (
  await getLibraryApi(sdkApi).getItemCollections(itemParameters)
).data;
assert.deepEqual(collections.Items, []);
assert.equal(collections.TotalRecordCount, 0);
const updatedData = (
  await userDataApi.updateItemUserData({
    ...itemParameters,
    updateUserItemDataDto: {
      IsFavorite: true,
      Played: false,
      PlaybackPositionTicks: 40_000_000,
    },
  })
).data;
assert.equal(updatedData.IsFavorite, true);
assert.equal(updatedData.PlaybackPositionTicks, 40_000_000);
assert.equal((await call(`/catalog/${direct.Id}/state`)).favorite, true);
assert.equal(
  (await userDataApi.getItemUserData(itemParameters)).data
    .PlaybackPositionTicks,
  40_000_000,
);
assert.equal(
  (await compat.get(`/Items/${direct.Id}`, { headers })).status(),
  200,
);
const still = await compat.get(`/Items/${direct.Id}/Images/Primary`, {
  headers,
});
assert.equal(still.status(), 200, 'A movie without artwork shows a still');
assert.equal(still.headers()['content-type'], 'image/jpeg');
assert.equal(
  (
    await compat.get(
      `/UserItems/${direct.Id}/UserData?UserId=00000000-0000-0000-0000-000000000000`,
      { headers },
    )
  ).status(),
  403,
);
for (const invalid of [
  { IsFavorite: false, Rating: 5 },
  { PlaybackPositionTicks: -1 },
  { IsFavorite: 'false' },
  { ItemId: '00000000-0000-0000-0000-000000000000', IsFavorite: false },
]) {
  assert.equal(
    (
      await compat.post(`/UserItems/${direct.Id}/UserData`, {
        headers,
        data: invalid,
      })
    ).status(),
    400,
  );
}
assert.equal(
  (await userDataApi.getItemUserData(itemParameters)).data.IsFavorite,
  true,
);
for (const suffix of ['UserData', 'Collections']) {
  assert.equal(
    (
      await compat.get(
        `/${suffix === 'UserData' ? 'UserItems' : 'Items'}/00000000-0000-0000-0000-000000000000/${suffix}`,
        { headers },
      )
    ).status(),
    404,
  );
}
await userDataApi.updateItemUserData({
  ...itemParameters,
  updateUserItemDataDto: {
    IsFavorite: originalData.IsFavorite,
    Played: false,
    PlaybackPositionTicks: 0,
  },
});
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
  const url = new URL(subtitle.DeliveryUrl, origin);
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
const originalPolicy = await call('/admin/settings');
try {
  await call('/admin/settings', 'PUT', {
    video_transcoding: false,
    audio_transcoding: false,
  });
  const policyResponse = await compat.get('/Users/Me', { headers });
  assert.equal(policyResponse.status(), 200);
  const disabledPolicy = (await policyResponse.json()).Policy;
  assert.equal(disabledPolicy.EnableVideoPlaybackTranscoding, false);
  assert.equal(disabledPolicy.EnableAudioPlaybackTranscoding, false);
  assert.equal(disabledPolicy.EnablePlaybackRemuxing, true);
  for (const [title, stream, status] of [
    ['Direct', true, 200],
    ['Remux', true, 200],
    ['Remux', false, 403],
    ['Audio Only', true, 403],
  ]) {
    const item = movies.Items.find((i) => i.Name === title);
    assert.ok(item);
    const response = await compat.post(`/Items/${item.Id}/PlaybackInfo`, {
      headers,
      data: {
        DeviceProfile: profile,
        EnableDirectPlay: title === 'Direct',
        EnableDirectStream: stream,
        EnableTranscoding: true,
      },
    });
    assert.equal(
      response.status(),
      status,
      `Disabled conversion: ${title}, direct stream ${stream}`,
    );
    if (response.ok()) {
      const session = await response.json();
      const url = session.MediaSources[0].DirectStreamUrl;
      assert.equal(
        (await compat.get(new URL(url, origin).toString())).status(),
        200,
      );
      await compat.delete(
        `/Videos/ActiveEncodings?PlaySessionId=${session.PlaySessionId}`,
        { headers },
      );
    }
  }
} finally {
  await call('/admin/settings', 'PUT', {
    video_transcoding: originalPolicy.video_transcoding,
    audio_transcoding: originalPolicy.audio_transcoding,
  });
}
for (const mode of ['remux', 'transcode', 'audio-only']) {
  const item = movies.Items.find(
    (i) => i.Name === (mode === 'audio-only' ? 'Audio Only' : 'Remux'),
  );
  assert.ok(item, 'Generate playback fixtures first');
  r = await compat.post(`/Items/${item.Id}/PlaybackInfo`, {
    headers,
    data: {
      DeviceProfile: profile,
      EnableDirectPlay: false,
      EnableDirectStream: ['remux', 'audio-only'].includes(mode),
      EnableTranscoding: true,
      SubtitleStreamIndex: -2,
    },
  });
  assert.equal(r.status(), 200, `${mode} playback info`);
  const session = await r.json();
  const converted = session.MediaSources[0];
  assert.equal(converted.SupportsDirectPlay, false);
  assert.equal(converted.SupportsTranscoding, true);
  assert.equal(converted.SupportsDirectStream, false);
  const playlistUrl = new URL(converted.TranscodingUrl, origin);
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
    if (mode === 'audio-only') {
      assert.ok(probe.streams.some((s) => s.codec_name === 'aac'));
      const hashes = (file) =>
        execFileSync(
          'ffmpeg',
          ['-v', 'error', '-i', file, '-map', '0:v:0', '-f', 'framemd5', '-'],
          { encoding: 'utf8', windowsHide: true },
        )
          .split('\n')
          .filter((line) => line && !line.startsWith('#'))
          .map((line) => line.split(',').at(-1).trim());
      const original = hashes(
        '.local/fixtures/playback/movies/Audio Only (2020).mkv',
      );
      const delivered = hashes(path);
      assert.ok(delivered.length > 12);
      const first = original.indexOf(delivered[0]);
      assert.ok(first >= 0);
      assert.deepEqual(
        delivered,
        original.slice(first, first + delivered.length),
        'Jellyfin audio conversion must preserve original video frames, including reverse segment requests',
      );
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
    process.env.COMPOSE_PROJECT_NAME ?? 'thelxinoe-compat',
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
      apiVersion: publicInfo.Version,
      sdkApiVersion: API_VERSION,
      currentSdkUserDataRoundTrip: true,
      approvedSingleUseQuickConnect: true,
      filterLanguagesAndCollectionReads: true,
      rejectedInvalidUpdatesWithoutMutation: true,
      views: views.Items.map((i) => i.Name),
      libraryPreferences,
      movies: movies.Items.map((i) => i.Name),
      compatibilityTokenRejectedByFirstParty: true,
      grantedDirectPlaybackAndProgress: true,
      revokedStreamRejected: true,
      queryCredentialsAbsentFromLogs: true,
      seekableRemuxAndTranscode: true,
      serverTranscodingPolicyEnforced: true,
      separateTranscodingPermissions: true,
      remuxWithTranscodingDisabled: true,
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
