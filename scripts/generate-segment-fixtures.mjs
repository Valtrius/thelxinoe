// Synthetic audio/video only. Common music occurs at different offsets per episode.
import { mkdirSync, writeFileSync, existsSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { execFileSync } from 'node:child_process';
const root = resolve('.local/segments/data/shows/Segment Fixture/Season 01');
const audioRoot = resolve('.local/segments/audio');
mkdirSync(root, { recursive: true });
mkdirSync(audioRoot, { recursive: true });
const rate = 11025,
  duration = 240;
function music(t, seed) {
  const note = Math.floor(t / 0.55),
    base = 130.81 * 2 ** (((note * 7 + seed) % 24) / 12);
  return (
    0.23 * Math.sin(2 * Math.PI * base * t) +
    0.14 * Math.sin(2 * Math.PI * base * 1.5 * t) +
    0.1 * Math.sin(2 * Math.PI * base * 2 * t)
  );
}
for (const episode of [1, 2]) {
  const target = `Segment Fixture - S01E0${episode}.mp4`;
  if (existsSync(join(root, target))) continue;
  const bytes = Buffer.alloc(44 + rate * duration * 2);
  bytes.write('RIFF');
  bytes.writeUInt32LE(bytes.length - 8, 4);
  bytes.write('WAVEfmt ', 8);
  bytes.writeUInt32LE(16, 16);
  bytes.writeUInt16LE(1, 20);
  bytes.writeUInt16LE(1, 22);
  bytes.writeUInt32LE(rate, 24);
  bytes.writeUInt32LE(rate * 2, 28);
  bytes.writeUInt16LE(2, 32);
  bytes.writeUInt16LE(16, 34);
  bytes.write('data', 36);
  bytes.writeUInt32LE(bytes.length - 44, 40);
  const intro = episode === 1 ? 12 : 22,
    credits = episode === 1 ? 205 : 198;
  for (let i = 0; i < rate * duration; i++) {
    const t = i / rate;
    const value =
      t >= intro && t < intro + 30
        ? music(t - intro, 3)
        : t >= credits && t < credits + 30
          ? music(t - credits, 11)
          : music(t, episode * 13 + 29) * 0.15;
    bytes.writeInt16LE(Math.round(value * 28000), 44 + i * 2);
  }
  writeFileSync(join(audioRoot, `episode-${episode}.wav`), bytes);
  try {
    execFileSync(
      'docker',
      [
        'run',
        '--rm',
        '--network',
        'none',
        '--entrypoint',
        'ffmpeg',
        '-v',
        `${root}:/fixtures:rw`,
        '-v',
        `${audioRoot}:/audio:ro`,
        'thelxinoe-server:0.1.0',
        '-nostdin',
        '-hide_banner',
        '-loglevel',
        'error',
        '-f',
        'lavfi',
        '-i',
        'color=c=navy:s=320x180:r=24',
        '-i',
        `/audio/episode-${episode}.wav`,
        '-t',
        String(duration),
        '-c:v',
        'libx264',
        '-preset',
        'ultrafast',
        '-c:a',
        'aac',
        '-b:a',
        '96k',
        '-movflags',
        '+faststart',
        '-y',
        `/fixtures/${target}`,
      ],
      { stdio: 'pipe', windowsHide: true },
    );
  } catch {
    throw Error('Generated segment fixture encoding failed');
  }
}
console.log(
  'Two generated episodes ready with shifted recurring intros and credits',
);
