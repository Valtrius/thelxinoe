// Use a signed-in, isolated desktop test profile with WebView2 debugging enabled.
// Results contain timings only. MPV configuration and plugin selection are kept.
import { chromium } from '@playwright/test';
import { writeFileSync } from 'node:fs';

const cdp = process.env.THELXINOE_BENCHMARK_CDP;
const origin = process.env.THELXINOE_BENCHMARK_URL;
if (!cdp || !origin)
  throw new Error('Set THELXINOE_BENCHMARK_CDP and THELXINOE_BENCHMARK_URL');
const browser = await chromium.connectOverCDP(cdp);
const page = browser
  .contexts()[0]
  .pages()
  .find((page) => page.url().startsWith('http://tauri.localhost'));
if (!page) throw new Error('No native desktop test window');
const invoke = (command, args = {}) =>
  page.evaluate(
    ({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args),
    { command, args },
  );
const api = async (path) => {
  const response = await invoke('backend_request', {
    path,
    method: 'GET',
    body: null,
  });
  if (response.status >= 400)
    throw new Error(`${path.split('?')[0]}: HTTP ${response.status}`);
  return response.body;
};
const results = [];
let started = false;
try {
  if ((await invoke('server_url')) !== origin)
    throw new Error('The selected desktop is connected to a different server');
  if ((await invoke('mpv_state')).status !== 'stopped')
    throw new Error('Stop playback in the isolated test desktop first');
  await api('/auth/me');
  const twitch = (await api('/online/twitch/feed')).items[0];
  const video = (await api('/online/youtube/feed')).items.find((video) =>
    process.env.THELXINOE_BENCHMARK_VIDEO
      ? video.id === process.env.THELXINOE_BENCHMARK_VIDEO
      : video.duration > 120 &&
        !['live', 'upcoming'].includes(video.broadcast) &&
        !video.download,
  );
  if (!video || !twitch) throw new Error('Missing public provider fixtures');
  const choices = [
    ['twitch', { id: `twitch:${twitch.id}`, title: twitch.display_name }],
    ['youtube', { id: `youtube:${video.id}`, title: video.title }],
  ];
  for (let repeat = 0; repeat < 3; repeat++) {
    for (const [provider, choice] of choices) {
      const start = performance.now();
      started = true;
      await invoke('mpv_play', {
        choice: { ...choice, fileId: null, position: 0 },
        queue: null,
        music: false,
      });
      const prepared = performance.now() - start;
      for (;;) {
        const state = await invoke('mpv_state');
        if (state.status === 'failed') throw new Error(state.error);
        if (state.video_ready && state.position > 0) break;
        if (performance.now() - start > 45000)
          throw new Error('MPV playback readiness timeout');
        await new Promise((resolve) => setTimeout(resolve, 50));
      }
      const result = {
        provider,
        repeat,
        prepared_ms: Math.round(prepared),
        video_ready_ms: Math.round(performance.now() - start),
      };
      await new Promise((resolve) => setTimeout(resolve, 1500));
      const state = await invoke('mpv_state');
      if (
        !state.video_ready ||
        state.status !== 'playing' ||
        state.position < 1
      )
        throw new Error('Native playback did not advance');
      results.push(result);
      console.log(JSON.stringify(result));
      await invoke('mpv_command', { command: 'stop', value: null });
    }
  }
} finally {
  writeFileSync(
    process.env.THELXINOE_BENCHMARK_OUTPUT ?? '.local/native-player.json',
    JSON.stringify(results, null, 2),
  );
  if (started)
    await invoke('mpv_command', { command: 'stop', value: null }).catch(
      () => {},
    );
  await browser.close();
}
