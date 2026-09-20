// Creates synthetic media and imports it only into compose.acquisition.test.yaml.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
function ffmpeg(args) {
  execFileSync(
    'docker',
    [
      'compose',
      '-f',
      'compose.acquisition.test.yaml',
      'exec',
      '-T',
      'server',
      'ffmpeg',
      '-nostdin',
      '-hide_banner',
      '-loglevel',
      'error',
      ...args,
    ],
    { stdio: 'pipe', windowsHide: true },
  );
}
const video = [
  '-f',
  'lavfi',
  '-i',
  'testsrc2=size=320x180:rate=24',
  '-f',
  'lavfi',
  '-i',
  'sine=frequency=440',
  '-t',
  '12',
  '-c:v',
  'libx264',
  '-preset',
  'ultrafast',
  '-c:a',
  'aac',
  '-y',
];
for (const name of [
  'The.Matrix.1999.1080p.WEB-DL.mkv',
  'Firefly.S01E01.1080p.WEB-DL.mkv',
])
  ffmpeg([...video, `/media/downloads/${name}`]);
ffmpeg([
  '-f',
  'lavfi',
  '-i',
  'sine=frequency=440',
  '-t',
  '12',
  '-c:a',
  'flac',
  '-metadata',
  'artist=Electric Light Orchestra',
  '-metadata',
  'album=Discovery',
  '-metadata',
  'title=Shine a Little Love',
  '-metadata',
  'track=1',
  '-y',
  '/media/downloads/01 Shine a Little Love.flac',
]);
for (const [kind, port, version] of [
  ['radarr', 27878, 3],
  ['sonarr', 28989, 3],
  ['lidarr', 28686, 1],
]) {
  const key = readFileSync(
    `.local/acquisition/${kind}/config.xml`,
    'utf8',
  ).match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
  async function api(path, method = 'GET', body) {
    const r = await fetch(`http://127.0.0.1:${port}/api/v${version}/${path}`, {
      method,
      headers: { 'X-Api-Key': key, 'Content-Type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined,
    });
    if (!r.ok) throw Error(`${kind} ${path}: HTTP ${r.status}`);
    return r.json();
  }
  let item;
  if (kind === 'radarr') {
    const movie = (await api('movie')).find((m) => m.tmdbId === 603);
    if (!movie) throw Error('Run test-acquisition.mjs first');
    const files = await api(
      'manualimport?folder=%2Fdata%2Fdownloads&filterExistingFiles=false',
    );
    const file = files.find((f) =>
      f.path.endsWith('The.Matrix.1999.1080p.WEB-DL.mkv'),
    );
    if (!file) throw Error('Movie fixture was not discovered');
    item = {
      path: file.path,
      movieId: movie.id,
      quality: file.quality,
      languages: file.languages,
    };
  } else if (kind === 'sonarr') {
    const series = (await api('series')).find((s) => s.tvdbId === 78874);
    if (!series) throw Error('Run test-acquisition.mjs first');
    const episode = (await api(`episode?seriesId=${series.id}`)).find(
      (e) => e.seasonNumber === 1 && e.episodeNumber === 2,
    );
    item = {
      path: '/media/downloads/Firefly.S01E01.1080p.WEB-DL.mkv',
      seriesId: series.id,
      episodeIds: [episode.id],
      quality: {
        quality: { id: 3, name: 'WEBDL-1080p' },
        revision: { version: 1, real: 0, isRepack: false },
      },
      languages: [{ id: 1, name: 'English' }],
    };
  } else {
    const album = (await api('album')).find(
      (a) => a.foreignAlbumId === 'e4f25e55-94d2-3037-a826-b3610490ea2d',
    );
    if (!album)
      throw Error(
        'Run test-acquisition.mjs with its default Discovery lookup first',
      );
    const track = (await api(`track?albumId=${album.id}`)).find(
      (t) => String(t.trackNumber) === '1',
    );
    item = {
      path: '/media/downloads/01 Shine a Little Love.flac',
      artistId: album.artistId,
      albumId: album.id,
      albumReleaseId: album.releases.find((r) => r.monitored).id,
      trackIds: [track.id],
      quality: {
        quality: { id: 7, name: 'FLAC' },
        revision: { version: 1, real: 0, isRepack: false },
      },
      disableReleaseSwitching: true,
    };
  }
  const command = await api('command', 'POST', {
    name: 'ManualImport',
    importMode: 'move',
    files: [item],
  });
  for (let attempt = 0; attempt < 120; attempt++) {
    const status = await api(`command/${command.id}`);
    if (status.status === 'completed') break;
    if (status.status === 'failed')
      throw Error(`${kind} fixture import failed`);
    if (attempt === 119) throw Error(`${kind} fixture import timed out`);
    await new Promise((r) => setTimeout(r, 1000));
  }
  console.log(`${kind}: generated fixture imported`);
}
