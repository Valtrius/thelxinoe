import { chromium, expect } from '@playwright/test';
import { writeFileSync, readdirSync } from 'node:fs';
import net from 'node:net';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
const page = browser
  .contexts()[0]
  .pages()
  .find((p) => p.url().startsWith('http://tauri.localhost'));
if (!page)
  throw Error(
    'Start the production Windows test app with WebView debugging on port 9223',
  );
async function native(command, args = {}) {
  return page.evaluate(
    ({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args),
    { command, args },
  );
}
async function api(path, method = 'GET', body = null) {
  const r = await native('backend_request', { path, method, body });
  if (r.status >= 400) throw Error(`${path}: ${r.body.error?.message}`);
  return r.body;
}
let preferences;
try {
  if (
    await page
      .getByRole('button', { name: 'Sign out', exact: true })
      .isVisible()
  )
    await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await page
    .getByLabel('Server address', { exact: true })
    .fill('http://127.0.0.1:19292');
  await page
    .getByRole('button', { name: 'Connect to server', exact: true })
    .click();
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('test-only long passphrase');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Sign out', exact: true }),
  ).toBeVisible();
  preferences = await api('/me/segments');
  const episode = (await api('/catalog?kind=episode')).items.find(
    (e) => e.title === 'Segment Fixture · S01E01',
  );
  expect(episode).toBeTruthy();
  const segments = await api(`/catalog/${episode.id}/segments`);
  const intro = segments.items.find((s) => s.kind === 'Intro');
  expect(intro).toBeTruthy();
  await page
    .getByRole('button', { name: 'Shows', exact: true })
    .first()
    .click();
  await page
    .locator('button.media-card')
    .filter({ hasText: 'Segment Fixture' })
    .click();
  const season = await api(`/catalog/${episode.parent_id}`);
  await page
    .locator('button.media-card')
    .filter({ hasText: season.title })
    .click();
  await page
    .locator('button.media-card')
    .filter({ hasText: episode.title })
    .click();
  const results = [];
  for (const mode of ['Ask', 'Auto', 'Ignore']) {
    const previousPipes = new Set(readdirSync('\\\\.\\pipe\\'));
    await api(`/catalog/${episode.id}/state`, 'PUT', { watched: false });
    await api('/me/segments', 'PUT', { ...preferences, Intro: mode });
    await page.getByRole('button', { name: 'Play media', exact: true }).click();
    await expect
      .poll(async () => (await native('mpv_state')).video_ready, {
        timeout: 30000,
      })
      .toBe(true);
    const view = await native('mpv_state');
    expect(view.file_id).toBe(segments.file_id);
    expect(view.generation).toBe(segments.generation);
    await native('mpv_command', { command: 'seek', value: intro.start + 1 });
    await expect(
      page.getByRole('region', { name: 'Native player' }),
    ).toHaveCount(0);
    if (mode === 'Ask') {
      await expect
        .poll(async () => (await native('mpv_state')).position)
        .toBeGreaterThanOrEqual(intro.start);
      const pipe = readdirSync('\\\\.\\pipe\\').find(
        (name) => name.startsWith('thelxinoe-') && !previousPipes.has(name),
      );
      expect(pipe).toBeTruthy();
      await new Promise((resolve, reject) => {
        const socket = net.connect('\\\\.\\pipe\\' + pipe);
        socket.setTimeout(3000, () =>
          socket.destroy(new Error('MPV keypress timed out')),
        );
        socket.on('error', reject);
        socket.on('connect', () =>
          socket.write(
            JSON.stringify({
              command: ['keypress', 'Ctrl+ENTER'],
              request_id: 1,
            }) + '\n',
          ),
        );
        socket.on('data', (data) => {
          if (data.toString().includes('"request_id":1')) {
            socket.end();
            resolve();
          }
        });
      });
    }
    if (mode !== 'Ignore')
      await expect
        .poll(async () => (await native('mpv_state')).position, {
          timeout: 15000,
        })
        .toBeGreaterThanOrEqual(intro.end - 0.1);
    else {
      await expect
        .poll(async () => (await native('mpv_state')).position)
        .toBeGreaterThanOrEqual(intro.start);
      await expect(
        page.getByRole('button', { name: 'Skip intro', exact: true }),
      ).toHaveCount(0);
      expect((await native('mpv_state')).position).toBeLessThan(intro.end);
    }
    results.push({ mode, position: (await native('mpv_state')).position });
    if (mode === 'Ask')
      await page.screenshot({ path: '.local/native-segments.png' });
    await native('mpv_command', { command: 'stop', value: null });
    await expect(
      page.getByRole('region', { name: 'Native player' }),
    ).toHaveCount(0);
  }
  writeFileSync(
    '.local/native-segments-result.json',
    JSON.stringify(
      { passed: true, edition: segments.file_id, results },
      null,
      2,
    ),
  );
  console.log('Windows MPV segment Ask, Auto, Ignore and file identity passed');
} finally {
  await native('mpv_command', { command: 'stop', value: null }).catch(() => {});
  if (preferences) await api('/me/segments', 'PUT', preferences);
  await browser.close();
}
