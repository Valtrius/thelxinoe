// Generates public-domain synthetic media for the private acquisition fixture.
import { execFileSync } from 'node:child_process';
import {
  readFileSync,
  writeFileSync,
  mkdirSync,
  copyFileSync,
  statSync,
} from 'node:fs';
const compose = ['compose', '-f', 'compose.acquisition.test.yaml'];
const root = '.local/acquisition/news';
mkdirSync(root, { recursive: true });
const ff = (args) =>
  execFileSync(
    'docker',
    [
      ...compose,
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
const xml = (s) =>
  s.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('"', '&quot;');
function publish(title, names) {
  const dir = `${root}/${title}`;
  mkdirSync(dir, { recursive: true });
  const files = [];
  for (const name of names) {
    copyFileSync(`.local/acquisition/data/downloads/${name}`, `${dir}/${name}`);
    const size = statSync(`${dir}/${name}`).size;
    let segments = '';
    for (let offset = 0, part = 1; offset < size; offset += 500000, part++) {
      const n = Math.min(500000, size - offset);
      segments += `<segment bytes="${n}" number="${part}">${xml(`${title}/${name}`)}?${part}=${offset}:${n}</segment>`;
    }
    files.push(
      `<file poster="fixture" date="${Math.floor(Date.now() / 1000)}" subject="&quot;${xml(name)}&quot; yEnc"><groups><group>alt.binaries.test</group></groups><segments>${segments}</segments></file>`,
    );
  }
  writeFileSync(
    `${root}/${title}.nzb`,
    `<?xml version="1.0"?><nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">${files.join('')}</nzb>`,
  );
}
for (const [title, duration] of [
  ['The.Matrix.1999.1080p.WEB-DL.FIXTURE2', 8400],
  ['Firefly.S01E01.1080p.WEB-DL.FIXTURE2', 2700],
]) {
  const name = title + '.mkv';
  ff([
    '-f',
    'lavfi',
    '-i',
    'color=black:size=1920x1080:rate=1',
    '-f',
    'lavfi',
    '-i',
    'anullsrc=r=8000:cl=mono',
    '-t',
    String(duration),
    '-c:v',
    'libx264',
    '-preset',
    'ultrafast',
    '-c:a',
    'aac',
    '-b:a',
    '8k',
    '-y',
    `/media/downloads/${name}`,
  ]);
  publish(title, [name]);
}
const key = readFileSync('.local/acquisition/lidarr/config.xml', 'utf8').match(
  /<ApiKey>(.*?)<\/ApiKey>/,
)[1];
async function api(p) {
  const r = await fetch('http://localhost:28686/api/v1/' + p, {
    headers: { 'X-Api-Key': key },
  });
  if (!r.ok) throw Error('Lidarr fixture lookup failed');
  return r.json();
}
const album = (await api('album')).find(
  (a) => a.foreignAlbumId === 'e4f25e55-94d2-3037-a826-b3610490ea2d',
);
if (!album) throw Error('Request Discovery before generating fixtures');
const release = album.releases.find((r) => r.monitored);
const tracks = await api('track?albumId=' + album.id);
const names = [];
for (const track of tracks) {
  const name = String(track.trackNumber).padStart(2, '0') + '.flac';
  const tags = {
    artist: 'Electric Light Orchestra',
    album: 'Discovery',
    title: track.title,
    track: track.trackNumber,
    disc: '1',
    MUSICBRAINZ_ALBUMID: release.foreignReleaseId,
    MUSICBRAINZ_RELEASEGROUPID: album.foreignAlbumId,
  };
  ff([
    '-f',
    'lavfi',
    '-i',
    'anullsrc=r=44100:cl=stereo',
    '-t',
    String(track.duration / 1000),
    '-c:a',
    'flac',
    ...Object.entries(tags).flatMap(([k, v]) => ['-metadata', `${k}=${v}`]),
    '-y',
    `/media/downloads/${name}`,
  ]);
  names.push(name);
}
publish('Electric.Light.Orchestra-Discovery-1979-FLAC-FIXTURE2', names);
console.log('Published generated movie, episode and complete album fixtures.');
