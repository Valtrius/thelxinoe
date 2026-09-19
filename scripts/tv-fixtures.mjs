import { mkdirSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
const root = '.local/fixtures/playback/movies';
mkdirSync(root, { recursive: true });
function ffmpeg(args) {
  const result = spawnSync(
    'ffmpeg',
    ['-hide_banner', '-loglevel', 'error', '-y', ...args],
    { stdio: 'inherit', windowsHide: true },
  );
  if (result.status !== 0) throw new Error('TV fixture generation failed');
}
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
  '90',
  '-c:v',
  'libx264',
  '-bf',
  '0',
  '-g',
  '48',
  '-keyint_min',
  '48',
  '-sc_threshold',
  '0',
  '-pix_fmt',
  'yuv420p',
  '-c:a',
  'aac',
  `${root}/TV Remux (2026).avi`,
]);
ffmpeg([
  '-i',
  `${root}/TV Remux (2026).avi`,
  '-c:v',
  'ffv1',
  '-c:a',
  'flac',
  `${root}/TV Transcode (2026).mkv`,
]);
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
  '8',
  '-vf',
  'zscale=pin=bt709:tin=bt709:min=bt709:p=bt2020:t=smpte2084:m=bt2020nc:r=limited,format=yuv420p10le',
  '-c:v',
  'libx265',
  '-preset',
  'ultrafast',
  '-x265-params',
  'pools=1:frame-threads=1:log-level=error',
  '-color_primaries',
  'bt2020',
  '-color_trc',
  'smpte2084',
  '-colorspace',
  'bt2020nc',
  '-c:a',
  'aac',
  `${root}/TV HDR (2026).mkv`,
]);
for (const name of ['TV Remux', 'TV Transcode'])
  writeFileSync(
    `${root}/${name} (2026).eng.srt`,
    Array.from({ length: 9 }, (_, i) => {
      const time = (s) =>
        `00:${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')},000`;
      return `${i + 1}\n${time(i * 10)} --> ${time((i + 1) * 10)}\nTV subtitle validation ${i * 10}-${(i + 1) * 10}s\n`;
    }).join('\n'),
  );
console.log('TV remux, conversion, HDR, and subtitle fixtures ready.');
