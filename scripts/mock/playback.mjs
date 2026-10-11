import { execFile } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, rename } from 'node:fs/promises';
import { dirname } from 'node:path';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

const execute = promisify(execFile);
export const samplePath = new URL(
  '../../.local/mock/sample.mp4',
  import.meta.url,
);
const audioTrack = {
  id: 'audio-0',
  index: 0,
  kind: 'audio',
  codec: 'aac',
  language: 'eng',
  title: 'Preview audio',
  supported: true,
};
const probe = {
  format: { tags: {} },
  streams: [
    { codec_type: 'video', height: 360, avg_frame_rate: '12/1' },
    { codec_type: 'audio', tags: { language: 'eng' } },
  ],
};
export async function prepareSample() {
  if (existsSync(samplePath)) return;
  await mkdir(dirname(fileURLToPath(samplePath)), { recursive: true });
  const temporary = new URL(
    `../../.local/mock/sample-${process.pid}.mp4`,
    import.meta.url,
  );
  await execute(
    'ffmpeg',
    [
      '-hide_banner',
      '-loglevel',
      'error',
      '-f',
      'lavfi',
      '-i',
      'testsrc2=size=640x360:rate=12',
      '-f',
      'lavfi',
      '-i',
      'sine=frequency=220:sample_rate=44100',
      '-t',
      '90',
      '-c:v',
      'libx264',
      '-preset',
      'ultrafast',
      '-crf',
      '32',
      '-pix_fmt',
      'yuv420p',
      '-c:a',
      'aac',
      '-b:a',
      '48k',
      '-movflags',
      '+faststart',
      '-y',
      fileURLToPath(temporary),
    ],
    { windowsHide: true, timeout: 30000 },
  );
  await rename(temporary, samplePath);
}

export async function playbackApi(s, c) {
  const { path, method, body, json, fail } = c;
  const queue = path.match(/^\/me\/queue\/([^/]+)$/);
  if (queue) {
    s.queues[queue[1]] ??= {
      revision: 0,
      items: [],
      current_index: 0,
      position: 0,
    };
    const q = s.queues[queue[1]];
    if (method === 'PUT') {
      Object.assign(q, body);
      q.revision++;
    }
    return json({
      ...q,
      tracks: q.items
        .map((id) => s.library.find((item) => item.id === id))
        .filter(Boolean),
    });
  }
  const source = path.match(/^\/catalog\/([^/]+)\/playback$/);
  if (source) {
    const item = s.library.find((item) => item.id === source[1]);
    return json({
      sources: [
        {
          id: item?.fileId ?? `file-${source[1]}`,
          edition: item?.kind === 'movie' ? 'Theatrical' : '',
          duration: 90,
          video: item?.kind !== 'track',
          tracks: [audioTrack],
          probe,
          size: 4000000,
        },
      ],
      preferences: s.playbackPreferences,
      progress: item?.position
        ? [{ edition: '', position: 20, duration: 90 }]
        : [],
      watched: false,
    });
  }
  if (path === '/playback' && method === 'POST') {
    if (!existsSync(samplePath))
      return fail(
        503,
        'Install FFmpeg and restart dev:mock to preview playback.',
      );
    const item = s.library.find((item) => item.id === body.media_id);
    const playback = {
      id: `playback-${++s.sequence}`,
      media_id: body.media_id,
      file_id: item?.fileId ?? `file-${body.media_id}`,
      generation: 'preview',
      live: /^(twitch|kick):/.test(body.media_id),
      url: '/mock-media/sample.mp4',
      mode: 'direct',
      qualities: [
        { value: 'auto', label: 'Auto' },
        { value: '360p', label: '360p' },
      ],
      quality: body.options?.quality ?? 'auto',
      position: Math.min(body.position ?? (item?.position ? 20 : 0), 80),
      duration: 90,
      timeline_start: 0,
      video: item?.kind !== 'track',
      tracks: [audioTrack],
      subtitles: [
        {
          id: 'eng',
          language: 'eng',
          title: 'English',
          url: '/mock-media/english.vtt',
        },
      ],
      selected_subtitle: body.options?.subtitle === 'eng' ? 'eng' : null,
      options: body.options,
      probe,
      replay_gain: s.playbackPreferences.replay_gain,
    };
    s.playback[playback.id] = playback;
    return json(playback);
  }
  const action = path.match(/^\/playback\/([^/]+)\/(progress|seek)$/);
  if (action) {
    const p = s.playback[action[1]];
    if (!p) return fail(404, 'Playback not found');
    p.position = body.position ?? p.position;
    const item = s.library.find((item) => item.id === p.media_id);
    if (item) item.position = p.position;
    if (action[2] === 'seek')
      return json({
        ...p,
        position: body.position,
        url: '/mock-media/sample.mp4',
      });
    return json({ saved: true });
  }
  return false;
}
