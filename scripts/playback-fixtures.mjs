import { mkdirSync, writeFileSync, copyFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
const root = '.local/fixtures/playback';
for (const kind of ['movies', 'shows', 'music'])
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
for (const [name, codec, extra, audio = 'aac'] of [
  ['Direct (2020).mp4', 'libx264', ['-movflags', '+faststart']],
  ['Remux (2020).mkv', 'libx264', []],
  ['Transcode (2020).avi', 'mpeg4', []],
  ['Audio Only (2020).mkv', 'libx264', [], 'ac3'],
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
    audio,
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
for (const episode of ['S01E01', 'S01E02', 'S00E01'])
  copyFileSync(
    resolve(root, 'movies', 'Direct (2020).mp4'),
    resolve(root, 'shows', `Fixture Show.${episode}.mp4`),
  );
// A forced styled track whose font the file attaches: the renderer's own
// fallback font, which the server must return as the attachment.
writeFileSync(
  `${root}/styled.ass`,
  `[Script Info]
ScriptType: v4.00+
PlayResX: 320
PlayResY: 180

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Sign,Liberation Sans,24,&H0000FFFF,&H000000FF,&H00000000,&H00000000,1,0,0,0,100,100,0,0,1,2,0,8,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:17.00,Sign,,0,0,0,,{\\pos(160,40)}Styled fixture
`,
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
  '-i',
  `${root}/styled.ass`,
  '-map',
  '0:v',
  '-map',
  '1:a',
  '-map',
  '2:s',
  '-t',
  '18',
  '-c:v',
  'libx264',
  '-g',
  '48',
  '-pix_fmt',
  'yuv420p',
  '-c:a',
  'aac',
  '-c:s',
  'ass',
  '-metadata:s:a:0',
  'language=eng',
  '-metadata:s:s:0',
  'language=eng',
  '-disposition:s:0',
  'forced',
  '-attach',
  'frontend/node_modules/jassub/dist/default.woff2',
  '-metadata:s:t:0',
  'mimetype=font/woff2',
  resolve(root, 'movies', 'Styled (2020).mkv'),
]);
// FFmpeg encodes picture subtitles only from pictures, so a PGS stream draws a
// white box on a black picture from 4 s to 14 s, which FFmpeg also converts to
// a forced DVD track. Keyframes every 2 s put the box's start before the
// keyframe that a seek to 8 s reads from.
writeFileSync(
  `${root}/picture.sup`,
  pictureSubtitle({ width: 320, height: 180, box: [40, 120, 240, 40] }),
);
ffmpeg([
  '-f',
  'lavfi',
  '-i',
  'color=c=black:size=320x180:rate=24',
  '-f',
  'lavfi',
  '-i',
  'sine=frequency=440:sample_rate=48000',
  // The DVD track then ends where the next PGS display set clears the box.
  '-fix_sub_duration',
  '-itsoffset',
  '4',
  '-i',
  `${root}/picture.sup`,
  '-map',
  '0:v',
  '-map',
  '1:a',
  '-map',
  '2:s',
  '-map',
  '2:s',
  '-t',
  '18',
  '-c:v',
  'libx264',
  '-g',
  '48',
  '-pix_fmt',
  'yuv420p',
  '-c:a',
  'aac',
  '-c:s:0',
  'copy',
  '-c:s:1',
  'dvdsub',
  '-metadata:s:a:0',
  'language=eng',
  '-metadata:s:s:0',
  'language=eng',
  '-metadata:s:s:1',
  'language=eng',
  '-disposition:s:1',
  'forced',
  resolve(root, 'movies', 'Picture (2020).mkv'),
]);
// A season longer than the page some Jellyfin clients assume when they omit Limit.
const longEpisode = (number) =>
  resolve(
    root,
    'shows',
    `Long Show.S01E${String(number).padStart(3, '0')}.mp4`,
  );
ffmpeg([
  '-f',
  'lavfi',
  '-i',
  'testsrc2=size=64x36:rate=1',
  '-t',
  '1',
  '-c:v',
  'libx264',
  '-pix_fmt',
  'yuv420p',
  longEpisode(1),
]);
for (let number = 2; number <= 120; number++)
  copyFileSync(longEpisode(1), longEpisode(number));
// Specials, two seasons and a file holding two episodes, for series order and Next up.
for (const episode of ['S00E01', 'S01E01', 'S01E02', 'S02E01E02', 'S02E03'])
  copyFileSync(
    longEpisode(1),
    resolve(root, 'shows', `Order Show.${episode}.mp4`),
  );
console.log(
  'Generated direct, remux, transcode, edition, subtitle, styled, picture subtitle, gapless music, long-season and series-order fixtures.',
);

/** A PGS stream that shows one opaque white `box` and clears it after 10 s. */
function pictureSubtitle({ width, height, box: [x, y, w, h] }) {
  const u16 = (value) => [value >> 8, value & 255];
  const segment = (seconds, type, data) => {
    const header = Buffer.alloc(13);
    header.write('PG');
    header.writeUInt32BE(Math.round(seconds * 90000), 2);
    header[10] = type;
    header.writeUInt16BE(data.length, 11);
    return Buffer.concat([header, Buffer.from(data)]);
  };
  const composition = (state, objects) => [
    ...u16(width),
    ...u16(height),
    0x10,
    ...u16(state ? 0 : 1),
    state,
    0,
    0,
    objects.length,
    ...objects.flatMap(() => [...u16(0), 0, 0, ...u16(x), ...u16(y)]),
  ];
  const window = [1, 0, ...u16(x), ...u16(y), ...u16(w), ...u16(h)];
  // Each line is one run of palette entry 1, then the end of the line.
  const line = [0, 0xc0 | (w >> 8), w & 255, 1, 0, 0];
  const bitmap = [...u16(w), ...u16(h), ...Array(h).fill(line).flat()];
  const object = [
    ...u16(0),
    0,
    0xc0,
    bitmap.length >> 16,
    (bitmap.length >> 8) & 255,
    bitmap.length & 255,
    ...bitmap,
  ];
  return Buffer.concat([
    segment(0, 0x16, composition(0x80, [0])),
    segment(0, 0x17, window),
    segment(0, 0x14, [0, 0, 1, 235, 128, 128, 255]),
    segment(0, 0x15, object),
    segment(0, 0x80, []),
    segment(10, 0x16, composition(0, [])),
    segment(10, 0x17, window),
    segment(10, 0x80, []),
  ]);
}
