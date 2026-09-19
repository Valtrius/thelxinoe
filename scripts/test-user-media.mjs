import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const browser = await chromium.launch();
const owner = await browser.newContext({ ignoreHTTPSErrors: true });
const guest = await browser.newContext({ ignoreHTTPSErrors: true });
const second = await browser.newContext({ ignoreHTTPSErrors: true });
const page = await owner.newPage(),
  other = await guest.newPage();
const origin = 'https://localhost:20443';
let playlist, originalFlags, originalZone, movie;
async function api(context, path, method = 'GET', data) {
  const response = await context.request.fetch(`${origin}/api/v1${path}`, {
    method,
    headers: { 'X-Thelxinoe-Client': '1' },
    data,
  });
  if (!response.ok()) throw new Error(`${path}: ${response.status()}`);
  return response.json();
}
async function login(page, username) {
  await page.goto(origin);
  await page.getByLabel('Username', { exact: true }).fill(username);
  await page
    .getByLabel('Password', { exact: true })
    .fill('test-only long passphrase');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(page.getByText('Connected', { exact: true })).toBeVisible();
  await expect(
    page.getByRole('region', { name: 'Favorites', exact: true }),
  ).toBeVisible();
}
try {
  await login(page, 'admin');
  originalZone = (await api(owner, '/auth/me')).user.timezone;
  const users = (await api(owner, '/users')).items;
  if (!users.some((u) => u.username === 'state-user'))
    await api(owner, '/users', 'POST', {
      username: 'state-user',
      password: 'test-only long passphrase',
      role: 'user',
    });
  movie = (await api(owner, '/catalog?kind=movie')).items.find(
    (i) => i.title === 'Direct',
  );
  originalFlags = await api(owner, `/catalog/${movie.id}/state`);
  await api(owner, `/catalog/${movie.id}/state`, 'PUT', {
    favorite: false,
    watch_later: false,
  });
  await page.getByRole('button', { name: 'Movies', exact: true }).click();
  await page.getByRole('button', { name: 'Direct 2020', exact: true }).click();
  await page.getByRole('button', { name: 'Favorite', exact: true }).click();
  await page.getByRole('button', { name: 'Watch Later', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Remove from Watch Later', exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(
    page
      .getByRole('region', { name: 'Favorites', exact: true })
      .getByRole('button', { name: 'Direct', exact: true }),
  ).toBeVisible();
  await login(other, 'state-user');
  expect((await api(guest, `/catalog/${movie.id}/state`)).favorite).toBe(false);
  await expect(
    other
      .getByRole('region', { name: 'Favorites', exact: true })
      .getByRole('button', { name: 'Direct', exact: true }),
  ).toHaveCount(0);
  await page.getByRole('button', { name: 'Playlists', exact: true }).click();
  await page.getByRole('button', { name: 'New playlist', exact: true }).click();
  const name = `Shared music ${Date.now()}`;
  await page.getByLabel('Playlist name', { exact: true }).fill(name);
  await page.getByLabel('Find tracks', { exact: true }).fill('Gapless');
  await page
    .getByRole('button', { name: 'Search tracks', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Add Gapless 1', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Add Gapless 2', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Save playlist', exact: true })
    .click();
  await expect
    .poll(async () => {
      const item = (await api(owner, '/playlists')).items.find(
        (p) => p.name === name,
      );
      playlist = item?.id;
      return item?.count;
    })
    .toBe(2);
  await other.getByRole('button', { name: 'Playlists', exact: true }).click();
  await other.getByRole('button', { name, exact: true }).click();
  await expect(
    other.getByRole('button', { name: 'Save playlist', exact: true }),
  ).toHaveCount(0);
  await expect(
    other.getByRole('button', { name: 'Play playlist', exact: true }),
  ).toBeEnabled();
  const publicRow = other
    .locator('section')
    .filter({ has: other.getByRole('button', { name, exact: true }) });
  await publicRow
    .getByRole('button', { name: 'Favorite playlist', exact: true })
    .click();
  await expect
    .poll(async () => (await api(guest, `/playlists/${playlist}`)).favorite)
    .toBe(true);
  expect((await api(owner, `/playlists/${playlist}`)).favorite).toBe(false);
  await page
    .getByRole('button', { name: 'Play playlist', exact: true })
    .click();
  await expect(
    page.getByRole('region', { name: 'Music player', exact: true }),
  ).toBeVisible();
  const client = await page.evaluate(() =>
    localStorage.getItem('thelxinoe-client-id'),
  );
  await expect
    .poll(async () => (await api(owner, `/me/queue/${client}`)).completed, {
      timeout: 20000,
    })
    .toBe(true);
  expect((await api(owner, `/me/queue/${client}`)).current_index).toBe(1);
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await page.reload();
  await expect(
    page.getByRole('region', { name: 'Saved music queue', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Replay queue', exact: true }),
  ).toBeVisible();
  const another = await second.newPage();
  await login(another, 'admin');
  const otherClient = await another.evaluate(() =>
    localStorage.getItem('thelxinoe-client-id'),
  );
  expect(otherClient).not.toBe(client);
  expect((await api(second, `/me/queue/${otherClient}`)).items).toHaveLength(0);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByLabel('Display timezone', { exact: true })
    .fill('Europe/Paris');
  await page
    .getByRole('button', { name: 'Save display preferences', exact: true })
    .click();
  await expect(
    page.getByText('Display preferences saved.', { exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'History', exact: true }).click();
  await expect(
    page.getByText('Times shown in Europe/Paris.', { exact: true }),
  ).toBeVisible();
  await page
    .getByLabel('History scope', { exact: true })
    .selectOption({ label: 'All users' });
  await expect(
    page.getByRole('heading', { name: 'By user', exact: true }),
  ).toBeVisible();
  await other.getByRole('button', { name: 'History', exact: true }).click();
  await expect(other.getByLabel('History scope', { exact: true })).toHaveCount(
    0,
  );
  const denied = await guest.request.get(`${origin}/api/v1/admin/history`);
  expect(denied.status()).toBe(403);
  await page.screenshot({
    path: '.local/user-media-history.png',
    fullPage: true,
  });
  writeFileSync(
    '.local/user-media-result.json',
    JSON.stringify(
      {
        verified_at: new Date().toISOString(),
        private_favorites: true,
        shared_owned_playlists: true,
        independent_playlist_favorites: true,
        persistent_client_queues: true,
        history_permissions: true,
        user_timezone: true,
      },
      null,
      2,
    ),
  );
  console.log(
    'HTTPS user-state UI passed: private lists, owned shared playlists, music queue persistence, timezone and history access.',
  );
} finally {
  if (playlist)
    await api(owner, `/playlists/${playlist}`, 'DELETE').catch(() => {});
  if (originalFlags && movie)
    await api(owner, `/catalog/${movie.id}/state`, 'PUT', {
      favorite: originalFlags.favorite,
      watch_later: originalFlags.watch_later,
    }).catch(() => {});
  if (originalZone)
    await api(owner, '/me/preferences', 'PUT', {
      timezone: originalZone,
    }).catch(() => {});
  await browser.close();
}
