import { chromium, expect } from '@playwright/test';
import { readFileSync, writeFileSync, existsSync, unlinkSync } from 'node:fs';
import { resolve } from 'node:path';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let originalConfig, originalPreferences;
const page = browser
  .contexts()[0]
  .pages()
  .find((p) => p.url().startsWith('http://tauri.localhost'));
if (!page)
  throw new Error(
    'Start the Windows desktop with WebView2 debugging on port 9223',
  );
async function native(command, args = {}) {
  return page.evaluate(
    ({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args),
    { command, args },
  );
}
async function api(path, method = 'GET', body = null) {
  const response = await native('backend_request', { path, method, body });
  if (response.status >= 400)
    throw new Error(`${path}: ${response.body.error?.message}`);
  return response.body;
}
try {
  if (
    await page
      .getByRole('button', { name: 'Sign out', exact: true })
      .isVisible()
  )
    await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await page
    .getByLabel('Server address', { exact: true })
    .fill('http://127.0.0.1:18686');
  await page
    .getByRole('button', { name: 'Connect to server', exact: true })
    .click();
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('test-only long passphrase');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'MPV', exact: true })
    .click();
  originalConfig = (await native('mpv_settings')).text;
  const settings = await native('mpv_settings');
  if (!settings.selection.path) {
    await page
      .getByRole('button', { name: 'Install or update MPV', exact: true })
      .click();
    await expect(
      page.getByText('MPV installed and verified', { exact: true }),
    ).toBeVisible({ timeout: 180000 });
  }
  const selected = await native('mpv_settings');
  expect(selected.selection.version).toMatch(/^mpv /);
  expect(selected.selection.digest).toMatch(/^[a-f0-9]{64}$/);
  await page.getByRole('button', { name: 'Movies', exact: true }).click();
  await page.getByRole('button', { name: 'Direct 2020', exact: true }).click();
  await page.getByRole('button', { name: 'Play media', exact: true }).click();
  await expect
    .poll(
      async () =>
        Number(
          await page
            .getByLabel('Native playback position', { exact: true })
            .inputValue(),
        ),
      { timeout: 30000 },
    )
    .toBeGreaterThan(1);
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  await expect.poll(async () => (await native('mpv_state')).paused).toBe(true);
  await expect
    .poll(async () => (await native('mpv_state')).video_ready)
    .toBe(true);
  const paused = await native('mpv_state');
  expect(paused.status).not.toBe('failed');
  await page
    .getByLabel('Native playback position', { exact: true })
    .evaluate((el) => {
      el.value = '10';
      el.dispatchEvent(new Event('change', { bubbles: true }));
    });
  await expect
    .poll(async () => (await native('mpv_state')).position, { timeout: 10000 })
    .toBeGreaterThanOrEqual(9.8);
  await page.screenshot({
    path: '.local/desktop-native-player.png',
    fullPage: true,
  });
  await page.reload();
  await expect(
    page.getByRole('region', { name: 'Native player' }),
  ).toBeVisible();
  expect((await native('mpv_state')).position).toBeGreaterThanOrEqual(9.8);
  expect((await native('mpv_state')).paused).toBe(true);
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await expect(
    page.getByRole('region', { name: 'Native player' }),
  ).not.toBeVisible();
  const movie = (await api('/catalog?kind=movie')).items.find(
    (i) => i.title === 'Direct',
  );
  expect(
    (await api(`/catalog/${movie.id}/playback`)).progress[0].position,
  ).toBeGreaterThanOrEqual(9.8);
  console.log(
    'Windows MPV installation, native video, pause, seek and server progress passed.',
  );
  originalPreferences = await api('/playback/preferences');
  await api('/playback/preferences', 'PUT', {
    ...originalPreferences,
    quality: '2mbps',
  });
  await native('mpv_play', {
    choice: { id: movie.id, title: movie.title },
    queue: null,
    music: false,
  });
  await expect
    .poll(async () => (await native('mpv_state')).mode, { timeout: 30000 })
    .toBe('transcode');
  await expect
    .poll(async () => (await native('mpv_state')).video_ready, {
      timeout: 30000,
    })
    .toBe(true);
  await native('mpv_command', { command: 'pause', value: null });
  await expect.poll(async () => (await native('mpv_state')).paused).toBe(true);
  for (const position of [14, 3]) {
    await native('mpv_command', { command: 'seek', value: position });
    await expect
      .poll(
        async () => Math.abs((await native('mpv_state')).position - position),
        { timeout: 30000 },
      )
      .toBeLessThan(0.5);
    await expect
      .poll(
        async () => {
          const state = await native('mpv_state');
          return state.status === 'paused' && state.video_ready;
        },
        { timeout: 30000 },
      )
      .toBe(true);
  }
  await native('mpv_command', { command: 'stop', value: null });
  await api('/playback/preferences', 'PUT', originalPreferences);
  console.log(
    'Native MPV decodes server HLS and seeks forward and backward while paused.',
  );
  const pcm = resolve('.local/mpv-gapless.wav').replaceAll('\\', '/');
  if (existsSync(pcm)) unlinkSync(pcm);
  await native('mpv_configuration', {
    text: `${originalConfig}\nao=pcm\nao-pcm-file=${pcm}\naudio-format=s16\naudio-samplerate=48000\naudio-channels=mono\nvolume=100\n`,
  });
  await page.getByRole('button', { name: 'Music', exact: true }).click();
  await page
    .getByRole('button', { name: 'Gapless Artist artist', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Gapless Album album', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Gapless 1 track', exact: true })
    .click();
  await page.getByRole('button', { name: 'Play media', exact: true }).click();
  await expect
    .poll(async () => (await native('mpv_state')).count, { timeout: 30000 })
    .toBe(2);
  await expect
    .poll(async () => (await native('mpv_state')).status, { timeout: 30000 })
    .toBe('stopped');
  expect((await native('mpv_state')).index).toBe(1);
  const wave = readFileSync(pcm);
  expect(wave.toString('ascii', 0, 4)).toBe('RIFF');
  let offset = 12,
    data;
  while (offset + 8 < wave.length) {
    const name = wave.toString('ascii', offset, offset + 4),
      size = wave.readUInt32LE(offset + 4);
    if (name === 'data') {
      data = wave.subarray(offset + 8, offset + 8 + size);
      break;
    }
    offset += 8 + size + (size % 2);
  }
  expect(data).toBeTruthy();
  const samples = data.length / 2;
  expect(Math.abs(samples - 6 * 48000)).toBeLessThanOrEqual(2);
  let min = 1,
    max = -1;
  for (let i = 3 * 48000 - 64; i < 3 * 48000 + 64; i++) {
    const sample = data.readInt16LE(i * 2) / 32768;
    min = Math.min(min, sample);
    max = Math.max(max, sample);
  }
  expect(min).toBeGreaterThan(0.249);
  expect(max).toBeLessThan(0.251);
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await expect(
    page.getByRole('region', { name: 'Native player' }),
  ).not.toBeVisible();
  const tracks = (await api('/catalog?kind=track')).items;
  for (const track of tracks.filter((t) => t.title.startsWith('Gapless '))) {
    expect((await api(`/catalog/${track.id}/playback`)).watched).toBe(true);
  }
  console.log(
    'Native MPV gapless queue has a continuous PCM seam and -6.02 dB ReplayGain, with per-track server progress.',
  );
  const client = await page.evaluate(() =>
    localStorage.getItem('thelxinoe-client-id'),
  );
  const savedQueue = await api(`/me/queue/${client}`);
  expect(savedQueue.current_index).toBe(1);
  expect(savedQueue.completed).toBe(true);
  await page.reload();
  await expect(
    page.getByRole('region', { name: 'Saved music queue', exact: true }),
  ).toBeVisible();
  console.log(
    'Desktop music queue restores from server state after page reload.',
  );
  writeFileSync(
    '.local/native-playback-result.json',
    JSON.stringify(
      {
        verified_at: new Date().toISOString(),
        mpv_version: selected.selection.version,
        native_video: true,
        seek: true,
        server_progress: true,
        persistent_client_queue: true,
        hls_seek: true,
        gapless_samples: samples,
        replay_gain_db: -6.0206,
      },
      null,
      2,
    ),
  );
} finally {
  await native('mpv_command', { command: 'stop', value: null }).catch(() => {});
  if (originalConfig !== undefined)
    await native('mpv_configuration', { text: originalConfig }).catch(() => {});
  if (originalPreferences)
    await api('/playback/preferences', 'PUT', originalPreferences).catch(
      () => {},
    );
  await api('/auth/logout', 'POST').catch(() => {});
  await native('change_server', { value: 'http://127.0.0.1:8484' }).catch(
    () => {},
  );
  await page.reload().catch(() => {});
  await browser.close();
}
