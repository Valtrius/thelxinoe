import { readFileSync, writeFileSync, renameSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { resolve, sep } from 'node:path';
const raw = readFileSync('.local/acquisition/radarr/config.xml', 'utf8'),
  key = raw.match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
const movies = await (
  await fetch('http://localhost:27878/api/v3/movie', {
    headers: { 'X-Api-Key': key },
  })
).json();
const movie = movies.find((m) => m.tmdbId === 603);
if (!movie?.hasFile) throw Error('Movie has not imported');
const path = movie.movieFile.path;
if (!path.startsWith('/data/movies/'))
  throw Error('Unexpected fixture movie path');
const local = resolve('.local/acquisition/data', path.slice(6));
if (!local.startsWith(resolve('.local/acquisition/data') + sep))
  throw Error('Path outside fixture');
writeFileSync(
  '.local/acquisition/data/downloads/fixture.srt',
  '1\n00:00:01,000 --> 00:00:03,000\nGenerated subtitle fixture.\n',
);
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
    '-loglevel',
    'error',
    '-i',
    path,
    '-i',
    '/data/downloads/fixture.srt',
    '-map',
    '0',
    '-map',
    '1',
    '-c',
    'copy',
    '-metadata:s:s:0',
    'language=eng',
    '-y',
    path + '.subbed.mkv',
  ],
  { stdio: 'pipe' },
);
renameSync(local + '.subbed.mkv', local);
const bazkey = readFileSync(
  '.local/acquisition/bazarr/config/config.yaml',
  'utf8',
).match(/^\s+apikey:\s*['"]?([a-zA-Z0-9]+)/m)[1];
const headers = { 'X-API-KEY': bazkey };
await fetch('http://localhost:26767/api/system/tasks?taskid=update_movies', {
  method: 'POST',
  headers,
});
console.log(
  'Embedded generated English subtitles and scheduled Bazarr synchronization.',
);
