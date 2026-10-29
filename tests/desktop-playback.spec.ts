import { expect, test, type Page } from '@playwright/test';
import { current } from './helpers/versions';
import { installUiFixture } from './helpers/ui-fixture';

type Played = { run: string; choice: { id: string; title: string } };
type Bridge = {
  played: Played[];
  commands: { command: string; run?: string | null }[];
  emitDesktopEvent: (name: string, payload: unknown) => void;
};

// The desktop shell with MPV runs answered by the test.
async function desktopFixture(page: Page) {
  const fixture = await installUiFixture(page);
  await page.addInitScript((version) => {
    const bridge = window as unknown as Bridge & {
      isTauri: boolean;
      __TAURI_INTERNALS__: unknown;
      __TAURI_EVENT_PLUGIN_INTERNALS__: unknown;
    };
    bridge.isTauri = true;
    bridge.played = [];
    bridge.commands = [];
    bridge.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    let callback = 0;
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    bridge.emitDesktopEvent = (event, payload) => {
      for (const id of listeners.get(event) ?? [])
        callbacks.get(id)?.({ event, id: 1, payload });
    };
    bridge.__TAURI_INTERNALS__ = {
      metadata: {
        currentWindow: { label: 'main' },
        currentWebview: { label: 'main' },
      },
      transformCallback: (handler: (event: unknown) => void) => {
        callbacks.set(++callback, handler);
        return callback;
      },
      unregisterCallback() {},
      async invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'server_url') return location.origin;
        if (command === 'backend_request') {
          const response = await fetch(`/api/v1${args.path}`, {
            method: String(args.method),
            headers: { 'Content-Type': 'application/json' },
            ...(args.body ? { body: JSON.stringify(args.body) } : {}),
          });
          return { status: response.status, body: await response.json() };
        }
        if (command === 'mpv_sessions') return [];
        if (command === 'mpv_play') {
          const played = args as unknown as Played;
          bridge.played.push(played);
          return {
            run: played.run,
            media_id: played.choice.id,
            title: played.choice.title,
            status: 'starting',
            music: false,
            video_ready: false,
          };
        }
        if (command === 'mpv_command') {
          bridge.commands.push(args as Bridge['commands'][number]);
          return;
        }
        if (command === 'tools_get') return { tools: [], mpv: {} };
        if (command === 'plugin:app|version') return version;
        if (command === 'desktop_update_status') return null;
        if (command === 'plugin:event|listen') {
          const event = String(args.event);
          listeners.set(event, [
            ...(listeners.get(event) ?? []),
            Number(args.handler),
          ]);
          return ++callback;
        }
        return false;
      },
    };
  }, current);
  // Next up cards play on a click.
  await page.route('**/api/v1/me/home', (route) =>
    route.fulfill({
      json: {
        continue_watching: [],
        next_up: [0, 1, 2].map((index) => ({
          id: `movie-${index}`,
          kind: 'movie',
          title: `Fixture movie ${index}`,
          available: true,
        })),
        watch_later: [],
        favorites: [],
      },
    }),
  );
  return fixture;
}

const played = (page: Page) =>
  page.evaluate(() => (window as unknown as Bridge).played);
async function report(
  page: Page,
  run: Played,
  status: string,
  extra: Record<string, unknown> = {},
) {
  await page.evaluate(
    ({ run, status, extra }) =>
      (window as unknown as Bridge).emitDesktopEvent('mpv-state', {
        run: run.run,
        media_id: run.choice.id,
        title: run.choice.title,
        status,
        music: false,
        video_ready: false,
        error: null,
        ...extra,
      }),
    { run, status, extra },
  );
}
async function play(page: Page, title: string) {
  const card = page.locator('article').filter({
    has: page.getByRole('heading', { name: title, exact: true }),
  });
  await card.locator('.card-primary').click();
  return card;
}

test('desktop videos start on their card and several play at once', async ({
  page,
}, testInfo) => {
  const fixture = await desktopFixture(page);
  await page.goto('/');
  const first = await play(page, 'Fixture movie 0');
  const starting = first.getByRole('status', {
    name: 'Starting Fixture movie 0',
  });
  await expect(starting).toBeVisible();
  await testInfo.attach('desktop-video-starting', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  // Nothing opens above the page while MPV starts.
  await expect(page.getByLabel('Native player')).toHaveCount(0);
  await expect(page.getByText('Loading player…')).toHaveCount(0);
  await expect.poll(async () => (await played(page)).length).toBe(1);
  const [run] = await played(page);
  await report(page, run, 'playing');
  // MPV is playing, but the spinner waits for its first video frame.
  await expect(starting).toBeVisible();
  await report(page, run, 'playing', { video_ready: true });
  await expect(starting).toHaveCount(0);

  const second = await play(page, 'Fixture movie 1');
  await expect(
    second.getByRole('status', { name: 'Starting Fixture movie 1' }),
  ).toBeVisible();
  await expect.poll(async () => (await played(page)).length).toBe(2);
  const next = (await played(page))[1];
  expect(next.run).not.toBe(run.run);
  await report(page, next, 'playing', { video_ready: true });
  await expect(second.getByRole('status')).toHaveCount(0);
  // Starting another video leaves the first window playing.
  expect(
    await page.evaluate(() => (window as unknown as Bridge).commands),
  ).toEqual([]);

  // A video that is already open is not started again.
  await play(page, 'Fixture movie 0');
  await expect(page.getByLabel('Native player')).toHaveCount(0);
  expect(await played(page)).toHaveLength(2);

  // A window that fails after it started still explains why.
  await report(page, run, 'failed', {
    error: 'The server connection was interrupted',
  });
  await expect(
    page.getByRole('alert').filter({ hasText: 'Fixture movie 0 stopped' }),
  ).toContainText('The server connection was interrupted');
  expect(fixture.unexpected).toEqual([]);
});

test('a desktop video that fails to start explains why without a player panel', async ({
  page,
}) => {
  await desktopFixture(page);
  await page.goto('/');
  const card = await play(page, 'Fixture movie 2');
  await expect.poll(async () => (await played(page)).length).toBe(1);
  const [run] = await played(page);
  await report(page, run, 'failed', {
    error: 'MPV could not decode or retrieve this media',
  });
  const toast = page.getByRole('alert').filter({
    hasText: 'Could not play Fixture movie 2',
  });
  await expect(toast).toContainText(
    'MPV could not decode or retrieve this media',
  );
  await expect(card.getByRole('status')).toHaveCount(0);
  await expect(page.getByLabel('Native player')).toHaveCount(0);
  await toast.getByRole('button', { name: 'Retry' }).click();
  await expect.poll(async () => (await played(page)).length).toBe(2);
  await expect(toast).toHaveCount(0);
});
