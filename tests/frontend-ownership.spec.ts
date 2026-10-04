import { expect, test, type WebSocketRoute } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';
import { responseGate } from './helpers/response-gate';

test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: 'ignoreErrors' });
});

test('preference events before a save response preserve the next queued edit', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page);
  const held = responseGate();
  let socket: WebSocketRoute | undefined;
  let preferences = {
    timezone: 'UTC',
    timezone_override: null as string | null,
    server_timezone: 'UTC',
    time_format: '24h',
    time_format_override: null as string | null,
    server_time_format: '24h',
  };
  const writes: unknown[] = [];
  let userReads = 0;
  await page.routeWebSocket('**/api/v1/events**', (ws) => {
    socket = ws;
  });
  await page.route('**/api/v1/auth/event-ticket', (route) =>
    route.fulfill({ json: { ticket: 'fixture', cursor: 0, epoch: 'fixture' } }),
  );
  await page.route('**/api/v1/auth/me', async (route) => {
    userReads++;
    await route.fallback();
  });
  await page.route('**/api/v1/me/preferences', async (route) => {
    if (route.request().method() === 'PUT') {
      const body = route.request().postDataJSON();
      writes.push(body);
      preferences = {
        ...preferences,
        timezone_override: body.timezone,
        timezone: body.timezone ?? 'UTC',
        time_format_override: body.time_format,
        time_format: body.time_format ?? '24h',
      };
      const saved = { ...preferences };
      if (writes.length === 1) await held.promise;
      return route.fulfill({ json: saved });
    }
    return route.fulfill({ json: preferences });
  });
  await page.goto('/');
  await expect.poll(() => Boolean(socket)).toBe(true);
  const zone = page.getByRole('combobox', {
    name: 'Display timezone',
    exact: true,
  });
  const format = page.getByRole('combobox', {
    name: 'Display time format',
    exact: true,
  });
  await zone.selectOption('Europe/Paris');
  await expect.poll(() => writes.length).toBe(1);
  await format.selectOption('12h');
  const before = userReads;
  socket!.send(
    JSON.stringify({
      id: 1,
      kind: 'preferences.changed',
      payload: preferences,
    }),
  );
  await expect.poll(() => userReads).toBeGreaterThan(before);
  await expect(zone).toHaveValue('Europe/Paris');
  await expect(format).toHaveValue('12h');
  const response = page.waitForResponse(
    (response) =>
      response.url().endsWith('/me/preferences') &&
      response.request().method() === 'PUT',
  );
  held.release();
  await (await response).finished();
  await expect.poll(() => writes.length).toBe(2);
  expect(writes).toEqual([
    { timezone: 'Europe/Paris', time_format: null },
    { timezone: 'Europe/Paris', time_format: '12h' },
  ]);
  await page.evaluate(() => new Promise(requestAnimationFrame));
  await format.focus();
  preferences = {
    ...preferences,
    timezone: 'America/New_York',
    timezone_override: 'America/New_York',
    time_format: '24h',
    time_format_override: '24h',
  };
  socket!.send(
    JSON.stringify({
      id: 2,
      kind: 'preferences.changed',
      payload: preferences,
    }),
  );
  await expect(zone).toHaveValue('America/New_York');
  await expect(format).toHaveValue('24h');
  await expect(format).toBeFocused();
  await testInfo.attach('preference-event-ordering', {
    body: JSON.stringify({ writes, preferences }),
    contentType: 'application/json',
  });
  await page.screenshot({
    path: testInfo.outputPath('queued-preferences-preserved.png'),
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('server-setting events cannot invalidate a save acknowledgement or block later remote updates', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'server',
  });
  const held = responseGate();
  let socket: WebSocketRoute | undefined;
  let settings = { timezone: 'UTC', time_format: '24h' };
  let reads = 0;
  const writes: unknown[] = [];
  await page.routeWebSocket('**/api/v1/events**', (ws) => {
    socket = ws;
  });
  await page.route('**/api/v1/auth/event-ticket', (route) =>
    route.fulfill({ json: { ticket: 'fixture', cursor: 0, epoch: 'fixture' } }),
  );
  await page.route('**/api/v1/admin/settings', async (route) => {
    if (route.request().method() === 'PUT') {
      writes.push(route.request().postDataJSON());
      if (writes.length > 1)
        return route.fulfill({
          status: 503,
          json: { error: { message: 'Save unavailable' } },
        });
      settings = route.request().postDataJSON();
      const saved = { ...settings };
      await held.promise;
      return route.fulfill({ json: { ok: true, ...saved } });
    }
    reads++;
    return route.fulfill({
      json: {
        ...settings,
        public_url: 'https://media.example',
        trusted_proxies: [],
      },
    });
  });
  await page.goto('/');
  await expect.poll(() => Boolean(socket)).toBe(true);
  const zone = page.getByRole('combobox', {
    name: 'Server default timezone',
    exact: true,
  });
  const format = page.getByRole('combobox', {
    name: 'Server default time format',
    exact: true,
  });
  await expect(format).toHaveValue('24h');
  await format.selectOption('12h');
  await expect.poll(() => writes.length).toBe(1);
  const before = reads;
  socket!.send(
    JSON.stringify({
      id: 1,
      kind: 'server.settings.changed',
      payload: settings,
    }),
  );
  await expect.poll(() => reads).toBeGreaterThan(before);
  settings = { timezone: 'Europe/Paris', time_format: '24h' };
  const beforeRemote = reads;
  socket!.send(
    JSON.stringify({
      id: 2,
      kind: 'server.settings.changed',
      payload: settings,
    }),
  );
  await expect.poll(() => reads).toBeGreaterThan(beforeRemote);
  await format.focus();
  const response = page.waitForResponse(
    (response) =>
      response.url().endsWith('/admin/settings') &&
      response.request().method() === 'PUT',
  );
  held.release();
  await (await response).finished();
  await expect(zone).toHaveValue('Europe/Paris');
  await expect(format).toHaveValue('24h');
  await expect(format).toBeFocused();
  await format.focus();
  settings = { timezone: 'America/New_York', time_format: '24h' };
  socket!.send(
    JSON.stringify({
      id: 3,
      kind: 'server.settings.changed',
      payload: settings,
    }),
  );
  await expect(zone).toHaveValue('America/New_York');
  await expect(format).toHaveValue('24h');
  await expect(format).toBeFocused();
  await format.selectOption('12h');
  await expect(page.getByRole('alert')).toContainText('Save unavailable');
  await expect(format).toHaveValue('24h');
  await expect(zone).toHaveValue('America/New_York');
  await testInfo.attach('server-setting-event-ordering', {
    body: JSON.stringify({ reads, writes, settings }),
    contentType: 'application/json',
  });
  await page.screenshot({
    path: testInfo.outputPath('acknowledged-server-defaults.png'),
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

for (const exit of ['replace', 'close', 'revoke'] as const) {
  test(`music teardown owns pending queue replacement on ${exit}`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, { section: 'Music' });
    const held = responseGate();
    const prepared: string[] = [],
      stopped: string[] = [];
    let heldStop = false,
      sequence = 0;
    const tracks = ['Q0', 'A', 'B'].map((id) => ({
      id,
      title: `Track ${id}`,
      kind: 'track',
      available: true,
      metadata: {},
    }));
    const wav = Buffer.alloc(44 + 8000 * 60 * 2);
    wav.write('RIFF');
    wav.writeUInt32LE(wav.length - 8, 4);
    wav.write('WAVEfmt ', 8);
    wav.writeUInt32LE(16, 16);
    wav.writeUInt16LE(1, 20);
    wav.writeUInt16LE(1, 22);
    wav.writeUInt32LE(8000, 24);
    wav.writeUInt32LE(16000, 28);
    wav.writeUInt16LE(2, 32);
    wav.writeUInt16LE(16, 34);
    wav.write('data', 36);
    wav.writeUInt32LE(wav.length - 44, 40);
    for (let i = 44; i < wav.length; i += 2)
      wav.writeInt16LE(
        Math.round(1000 * Math.sin(((i - 44) * Math.PI * 440) / 8000)),
        i,
      );
    await page.addInitScript(() => {
      const contexts: AudioContext[] = [];
      const Original = window.AudioContext;
      window.AudioContext = class extends Original {
        constructor(options?: AudioContextOptions) {
          super(options);
          contexts.push(this);
        }
      };
      Object.assign(window, { musicContexts: contexts });
    });
    await page.route('**/api/v1/catalog?**', (route) =>
      route.fulfill({ json: { items: tracks } }),
    );
    await page.route('**/api/v1/me/queue/**', (route) =>
      route.fulfill({
        json: {
          revision: ++sequence,
          items: [],
          current_index: 0,
          position: 0,
        },
      }),
    );
    await page.route('**/api/v1/catalog/*/playback', (route) =>
      route.fulfill({
        json: {
          sources: [
            {
              id: 'audio',
              size: wav.length,
              duration: 60,
              tracks: [{ kind: 'audio', codec: 'pcm_s16le' }],
            },
          ],
          preferences: { quality: 'original', replay_gain: 'off' },
          progress: [],
          watched: false,
        },
      }),
    );
    await page.route('**/api/v1/playback', (route) => {
      const body = route.request().postDataJSON();
      const id = `${body.media_id}-${prepared.length}`;
      prepared.push(body.media_id);
      return route.fulfill({
        json: {
          id,
          mode: 'direct',
          url: '/fixture/music.wav',
          position: 0,
          duration: 60,
          probe: {},
          replay_gain: 'off',
        },
      });
    });
    await page.route('**/fixture/music.wav', (route) =>
      route.fulfill({ contentType: 'audio/wav', body: wav }),
    );
    await page.route('**/api/v1/playback/*/**', async (route) => {
      const body = route.request().postDataJSON();
      if (body?.state === 'stopped') {
        stopped.push(new URL(route.request().url()).pathname);
        if (!heldStop) {
          heldStop = true;
          await held.promise;
        }
      }
      return route.fulfill({ json: { saved: true } });
    });
    await page.route('**/api/v1/playback/*', (route) =>
      route.fulfill({ json: { deleted: true } }),
    );
    await page.route('**/api/v1/auth/sessions/session-0', (route) =>
      route.fulfill({ json: { revoked: true } }),
    );
    await page.route('**/api/v1/auth/login', (route) =>
      route.fulfill({
        json: {
          user: {
            id: 'second-user',
            username: 'Second user',
            role: 'user',
            timezone: 'UTC',
          },
        },
      }),
    );
    await page.goto('/');
    await page
      .getByRole('button', { name: 'Play Track Q0', exact: true })
      .click();
    await expect(
      page.getByLabel('Music position', { exact: true }),
    ).toBeVisible();
    await expect.poll(() => prepared.length).toBe(2);
    await page
      .getByRole('button', { name: 'Play Track A', exact: true })
      .click();
    await expect.poll(() => heldStop).toBe(true);
    if (exit === 'replace') {
      await page
        .getByRole('button', { name: 'Play Track B', exact: true })
        .click();
      await expect(
        page.getByRole('heading', { name: 'Track B', exact: true }),
      ).toBeVisible();
    } else if (exit === 'close') {
      await page
        .getByRole('button', { name: 'Close player', exact: true })
        .click();
      await expect(
        page.getByRole('region', { name: 'Music player' }),
      ).toHaveCount(0);
    } else {
      await page.getByRole('link', { name: 'Settings', exact: true }).click();
      await page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('link', { name: 'Devices', exact: true })
        .click();
      await page
        .getByRole('button', { name: 'Revoke', exact: true })
        .first()
        .click();
      await page.getByLabel('Username', { exact: true }).fill('Second user');
      await page
        .getByLabel('Password', { exact: true })
        .fill('test passphrase');
      await page.getByRole('button', { name: 'Sign in', exact: true }).click();
      await expect(
        page.getByRole('button', { name: 'Sign out', exact: true }),
      ).toBeVisible();
    }
    const response = page.waitForResponse(
      (response) =>
        response.url().includes('/progress') &&
        response.request().postDataJSON()?.state === 'stopped',
    );
    held.release();
    await response;
    if (exit === 'replace') {
      await expect(
        page.getByLabel('Music position', { exact: true }),
      ).toBeVisible();
      await expect.poll(() => prepared).toEqual(['Q0', 'A', 'B']);
    } else
      await expect(
        page.getByRole('region', { name: 'Music player' }),
      ).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(() => {
          const contexts = (
            window as unknown as { musicContexts: AudioContext[] }
          ).musicContexts;
          return contexts.filter((context) => context.state !== 'closed')
            .length;
        }),
      )
      .toBe(exit === 'replace' ? 1 : 0);
    expect(prepared).toEqual(
      exit === 'replace' ? ['Q0', 'A', 'B'] : ['Q0', 'A'],
    );
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
    await testInfo.attach('music-lifetime', {
      body: JSON.stringify({ exit, prepared, stopped }),
      contentType: 'application/json',
    });
  });
}

test('a provider screen reads only its account and does not depend on sibling providers', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Twitch' });
  const reads: string[] = [];
  await page.route('**/api/v1/online/**', (route) => {
    const path = new URL(route.request().url()).pathname;
    reads.push(path);
    if (path.endsWith('/twitch/feed'))
      return route.fulfill({ json: { items: [] } });
    if (path.endsWith('/twitch'))
      return route.fulfill({
        json: {
          configured: true,
          account: { status: 'connected', display_name: 'Twitch viewer' },
        },
      });
    if (path.endsWith('/youtube') || path.endsWith('/kick'))
      return route.fulfill({
        status: 503,
        json: { error: { message: 'Sibling unavailable' } },
      });
    return route.fallback();
  });
  await page.goto('/');
  await expect(
    page.getByRole('button', { name: 'Refresh', exact: true }),
  ).toBeVisible();
  expect(reads.filter((path) => /\/(youtube|kick)(\/|$)/.test(path))).toEqual(
    [],
  );
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await testInfo.attach('provider-reads', {
    body: JSON.stringify(reads),
    contentType: 'application/json',
  });
});

test('switching providers can load the new account while the previous account request is held', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'YouTube' });
  const held = responseGate();
  let started = false;
  await page.route('**/api/v1/online/youtube', async (route) => {
    started = true;
    await held.promise;
    return route.fulfill({
      json: {
        configured: true,
        account: { status: 'disconnected', display_name: '' },
      },
    });
  });
  await page.route('**/api/v1/online/youtube/watchlists', (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route('**/api/v1/online/twitch', (route) =>
    route.fulfill({
      json: {
        configured: true,
        account: { status: 'connected', display_name: 'Twitch viewer' },
      },
    }),
  );
  await page.route('**/api/v1/online/twitch/feed', (route) =>
    route.fulfill({ json: { items: [] } }),
  );
  await page.goto('/');
  await expect.poll(() => started).toBe(true);
  await page.getByRole('link', { name: 'Twitch', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Refresh', exact: true }),
  ).toBeVisible();
  held.release();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await testInfo.attach('provider-switch', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

for (const exit of ['navigation', 'logout', 'revoke'] as const) {
  test(`queued display saves stop on ${exit}`, async ({ page }, testInfo) => {
    const fixture = await installUiFixture(page);
    const held = responseGate();
    const writes: unknown[] = [];
    await page.route('**/api/v1/me/preferences', async (route) => {
      if (route.request().method() !== 'PUT') return route.fallback();
      writes.push(route.request().postDataJSON());
      if (writes.length === 1) await held.promise;
      return route.fallback();
    });
    await page.route('**/api/v1/auth/sessions/session-0', (route) =>
      route.fulfill({ json: { revoked: true } }),
    );
    await page.route('**/api/v1/auth/logout', (route) =>
      route.fulfill({ json: { signed_out: true } }),
    );
    await page.route('**/api/v1/auth/login', (route) =>
      route.fulfill({
        json: {
          user: {
            id: 'second-user',
            username: 'Second user',
            role: 'user',
            timezone: 'UTC',
          },
        },
      }),
    );
    await page.goto('/');
    const format = page.getByRole('combobox', {
      name: 'Display time format',
      exact: true,
    });
    await format.selectOption('12h');
    await expect.poll(() => writes.length).toBe(1);
    await format.selectOption('24h');
    if (exit === 'logout') {
      await page.getByRole('button', { name: 'Sign out', exact: true }).click();
    } else {
      await page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('link', { name: 'Devices', exact: true })
        .click();
      if (exit === 'revoke')
        await page
          .getByRole('button', { name: 'Revoke', exact: true })
          .first()
          .click();
    }
    if (exit !== 'navigation') {
      await page.getByLabel('Username', { exact: true }).fill('Second user');
      await page
        .getByLabel('Password', { exact: true })
        .fill('test passphrase');
      await page.getByRole('button', { name: 'Sign in', exact: true }).click();
      await expect(
        page.getByRole('button', { name: 'Sign out', exact: true }),
      ).toBeVisible();
    }
    const response = page.waitForResponse(
      (response) =>
        response.url().endsWith('/me/preferences') &&
        response.request().method() === 'PUT',
    );
    held.release();
    await response;
    await page
      .getByRole('navigation', { name: 'Main navigation' })
      .getByRole('link', { name: 'Movies', exact: true })
      .click();
    await expect(
      page.getByRole('heading', { name: 'Movies', exact: true }),
    ).toBeVisible();
    expect(writes).toHaveLength(1);
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
    await testInfo.attach('mutation-ownership', {
      body: JSON.stringify({ exit, writes }),
      contentType: 'application/json',
    });
  });
}

test('server display defaults wait for hydration and roll back to the loaded baseline', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'server',
  });
  const held = responseGate();
  let requested = false;
  const writes: unknown[] = [];
  await page.route('**/api/v1/admin/settings', async (route) => {
    if (route.request().method() === 'PUT') {
      writes.push(route.request().postDataJSON());
      return route.fulfill({
        status: 503,
        json: { error: { message: 'Display settings unavailable' } },
      });
    }
    requested = true;
    await held.promise;
    return route.fulfill({
      json: { timezone: 'Europe/Paris', time_format: '12h' },
    });
  });
  await page.goto('/');
  await expect.poll(() => requested).toBe(true);
  await expect(
    page.getByRole('form', { name: 'Server display defaults' }),
  ).toHaveCount(0);
  expect(writes).toHaveLength(0);
  held.release();
  const zone = page.getByRole('combobox', {
    name: 'Server default timezone',
    exact: true,
  });
  const format = page.getByRole('combobox', {
    name: 'Server default time format',
    exact: true,
  });
  await expect(zone).toHaveValue('Europe/Paris');
  await expect(format).toHaveValue('12h');
  await format.selectOption('24h');
  await expect(page.getByRole('alert')).toContainText(
    'Display settings unavailable',
  );
  await expect(zone).toHaveValue('Europe/Paris');
  await expect(format).toHaveValue('12h');
  expect(writes).toEqual([{ timezone: 'Europe/Paris', time_format: '24h' }]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await testInfo.attach('loaded-display-baseline', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

for (const intent of ['select', 'close', 'domain'] as const) {
  test(`catalog refresh respects ${intent} after its detail request starts`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, { section: 'Movies' });
    const held = responseGate();
    let socket: WebSocketRoute | undefined;
    let calls = 0;
    await page.routeWebSocket('**/api/v1/events**', (ws) => {
      socket = ws;
    });
    await page.route('**/api/v1/auth/event-ticket', (route) =>
      route.fulfill({
        json: { ticket: 'fixture', cursor: 0, epoch: 'fixture' },
      }),
    );
    await page.route('**/api/v1/catalog/movie-**', async (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path.endsWith('/state'))
        return route.fulfill({
          json: { favorite: false, watched: false, watch_later: false },
        });
      const id = path.split('/').at(-1)!;
      if (id === 'movie-0' && ++calls === 2) await held.promise;
      return route.fulfill({
        json: {
          id,
          kind: 'movie',
          title: `Fixture movie ${id.split('-')[1]}`,
          available: true,
          metadata: {},
          files: [],
        },
      });
    });
    await page.goto('/');
    await page
      .getByRole('button', { name: 'Details Fixture movie 0', exact: true })
      .click();
    await expect(
      page.getByRole('heading', {
        name: 'Fixture movie 0',
        exact: true,
        level: 2,
      }),
    ).toBeVisible();
    await expect.poll(() => Boolean(socket)).toBe(true);
    socket!.send(
      JSON.stringify({ id: 1, kind: 'catalog.changed', payload: {} }),
    );
    await expect.poll(() => calls).toBe(2);
    if (intent === 'select')
      await page
        .getByRole('button', { name: 'Details Fixture movie 1', exact: true })
        .click();
    else if (intent === 'close')
      await page.getByRole('button', { name: 'Close', exact: true }).click();
    else await page.getByRole('link', { name: 'Shows', exact: true }).click();
    const response = page.waitForResponse((response) =>
      response.url().endsWith('/catalog/movie-0'),
    );
    held.release();
    await response;
    await expect(
      page.getByRole('heading', {
        name: 'Fixture movie 0',
        exact: true,
        level: 2,
      }),
    ).toHaveCount(0);
    if (intent === 'select')
      await expect(
        page.getByRole('heading', {
          name: 'Fixture movie 1',
          exact: true,
          level: 2,
        }),
      ).toBeVisible();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
    await testInfo.attach('catalog-selection', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
  });
}

test('playlist opens follow selection and saving locks the whole draft without redirecting a newer editor', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Playlists' });
  const opened = responseGate(),
    saved = responseGate();
  let holdingOpen = true,
    saving = false;
  const playlists = ['A', 'B'].map((id) => ({
    id,
    name: `Playlist ${id}`,
    description: '',
    owner_id: 'layout-fixture',
    owner: 'Viewer',
    favorite: false,
    count: 1,
    revision: 1,
    items: [{ id: 'track', kind: 'track', title: 'Track', available: true }],
  }));
  await page.route('**/api/v1/playlists**', async (route) => {
    const id = new URL(route.request().url()).pathname.split('/').at(-1);
    if (id === 'playlists')
      return route.fulfill({ json: { items: playlists } });
    if (route.request().method() === 'PUT') {
      saving = true;
      await saved.promise;
      return route.fulfill({ json: { id } });
    }
    if (id === 'A' && holdingOpen) await opened.promise;
    return route.fulfill({ json: playlists.find((p) => p.id === id) });
  });
  await page.goto('/');
  await page.getByRole('button', { name: 'Playlist A', exact: true }).click();
  await page.getByRole('button', { name: 'Playlist B', exact: true }).click();
  const name = page.getByLabel('Playlist name', { exact: true });
  await expect(name).toHaveValue('Playlist B');
  await name.fill('New B draft');
  const response = page.waitForResponse((response) =>
    response.url().endsWith('/playlists/A'),
  );
  holdingOpen = false;
  opened.release();
  await response;
  await expect(name).toHaveValue('New B draft');
  await page
    .getByRole('button', { name: 'Save playlist', exact: true })
    .click();
  await expect.poll(() => saving).toBe(true);
  await expect(name).toBeDisabled();
  await expect(
    page.getByRole('button', { name: 'Remove track 1', exact: true }),
  ).toBeDisabled();
  await page.getByRole('button', { name: 'Playlist A', exact: true }).click();
  await expect(name).toHaveValue('Playlist A');
  const saveResponse = page.waitForResponse(
    (response) => response.request().method() === 'PUT',
  );
  saved.release();
  await saveResponse;
  await expect(name).toHaveValue('Playlist A');
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await testInfo.attach('playlist-draft', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});
