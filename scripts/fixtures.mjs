import { mkdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
for (const folder of ['movies', 'shows', 'music'])
  mkdirSync(`.local/fixtures/${folder}`, { recursive: true });
const fixtures = [
  ['movies/Thelxinoe Fixture (2020).mp4', 440, []],
  ['movies/Thelxinoe Fixture (2020)-trailer.mp4', 660, []],
  ['shows/Fixture Show.S01E01E02.mp4', 880, []],
  ['shows/Fixture Show.S00E01.mp4', 990, []],
  [
    'music/01 - Fixture.flac',
    220,
    [
      '-metadata',
      'artist=Fixture Artist',
      '-metadata',
      'album=Fixture Album',
      '-metadata',
      'title=Fixture Track',
      '-metadata',
      'disc=1',
      '-metadata',
      'track=1',
      '-metadata',
      'REPLAYGAIN_TRACK_GAIN=-5.00 dB',
    ],
  ],
];
for (const [file, frequency, extra] of fixtures) {
  const video = file.endsWith('.mp4');
  const result = spawnSync(
    'ffmpeg',
    [
      '-hide_banner',
      '-loglevel',
      'error',
      '-y',
      ...(video
        ? ['-f', 'lavfi', '-i', 'color=c=0x29453f:s=320x180:r=24']
        : []),
      '-f',
      'lavfi',
      '-i',
      `sine=frequency=${frequency}:sample_rate=48000`,
      '-t',
      '2',
      ...(video
        ? [
            '-c:v',
            'libx264',
            '-pix_fmt',
            'yuv420p',
            '-c:a',
            'aac',
            '-movflags',
            '+faststart',
          ]
        : ['-c:a', 'flac']),
      ...extra,
      resolve('.local/fixtures', file),
    ],
    { stdio: 'inherit' },
  );
  if (result.status !== 0) throw new Error(`Failed to generate ${file}`);
}
console.log(
  'Generated synthetic movie, episode, trailer and tagged music fixtures in .local/fixtures',
);
