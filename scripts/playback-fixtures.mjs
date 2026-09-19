import { mkdirSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
const root = '.local/fixtures/playback';
for (const kind of ['movies', 'music'])
  mkdirSync(`${root}/${kind}`, { recursive: true });
function ffmpeg(args) {
  const result = spawnSync(
    'ffmpeg',
    ['-hide_banner', '-loglevel', 'error', '-y', ...args],
    { stdio: 'inherit', windowsHide: true },
  );
  if (result.status !== 0)
    throw new Error('Playback fixture generation failed');
}
for (const [name, codec, extra] of [
  ['Direct (2020).mp4', 'libx264', ['-movflags', '+faststart']],
  ['Remux (2020).mkv', 'libx264', []],
  ['Transcode (2020).avi', 'mpeg4', []],
  ['Edition (2020) {edition-Theatrical}.mp4', 'libx264', []],
  ['Edition (2020) {edition-Extended}.mp4', 'libx264', []],
])
  ffmpeg([
    '-f',
    'lavfi',
    '-i',
    'testsrc2=size=320x180:rate=24',
    '-f',
    'lavfi',
    '-i',
    'sine=frequency=440:sample_rate=48000',
    '-t',
    '18',
    '-c:v',
    codec,
    '-g',
    '48',
    '-pix_fmt',
    'yuv420p',
    '-c:a',
    'aac',
    ...extra,
    resolve(root, 'movies', name),
  ]);
writeFileSync(
  `${root}/movies/Direct (2020).eng.srt`,
  '1\n00:00:00,000 --> 00:00:17,000\nThelxinoe subtitle fixture\n',
);
ffmpeg([
  '-f',
  'lavfi',
  '-i',
  'testsrc2=size=320x180:rate=24',
  '-f',
  'lavfi',
  '-i',
  'sine=frequency=440:sample_rate=48000',
  '-f',
  'lavfi',
  '-i',
  'sine=frequency=880:sample_rate=48000',
  '-i',
  resolve(root, 'movies', 'Direct (2020).eng.srt'),
  '-map',
  '0:v',
  '-map',
  '1:a',
  '-map',
  '2:a',
  '-map',
  '3:s',
  '-t',
  '18',
  '-c:v',
  'libx264',
  '-g',
  '48',
  '-c:a',
  'aac',
  '-c:s',
  'mov_text',
  '-metadata:s:a:0',
  'language=eng',
  '-metadata:s:a:1',
  'language=fra',
  '-metadata:s:s:0',
  'language=eng',
  resolve(root, 'movies', 'Tracks (2020).mp4'),
]);
for (const number of [1, 2])
  ffmpeg([
    '-f',
    'lavfi',
    '-i',
    'aevalsrc=0.5:s=48000:d=3',
    '-c:a',
    'flac',
    '-metadata',
    'artist=Gapless Artist',
    '-metadata',
    'album=Gapless Album',
    '-metadata',
    `title=Gapless ${number}`,
    '-metadata',
    `track=${number}`,
    '-metadata',
    'REPLAYGAIN_TRACK_GAIN=-6.0206 dB',
    '-metadata',
    'REPLAYGAIN_TRACK_PEAK=0.5',
    resolve(root, 'music', `${number}.flac`),
  ]);
console.log(
  'Generated direct, remux, transcode, edition, subtitle and gapless music fixtures.',
);
