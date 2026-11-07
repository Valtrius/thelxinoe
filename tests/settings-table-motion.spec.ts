import { expect, test, type Page, type Route } from '@playwright/test';
import { pauseAnimations, seekAnimations } from './helpers/paused-animations';
import { responseGate } from './helpers/response-gate';
import { installUiFixture } from './helpers/ui-fixture';

const job = (id: string, target: string) => ({
  id,
  kind: 'library.scan',
  action: null,
  state: 'complete',
  attempts: 1,
  error: null,
  created_at: 1789984800,
  available_at: 1789984800,
  started_at: 1789984802,
  completed_at: 1789984871,
  target,
  service: null,
  automatic: null,
  actor: null,
});

const episode = (id: string, title: string) => ({
  media_id: id,
  file_id: `${id}-file`,
  title,
  status: 'found',
  error: null,
  analyzed_at: 1789984800,
  segments: [],
});

const entry = (id: number, target: string) => ({
  id,
  action: 'settings.update',
  actor_id: 'layout-fixture',
  actor: 'Layout viewer',
  target,
  created_at: 1789984800,
});

const coverage = {
  episodes: 3,
  intro: 1,
  credits: 1,
  nothing: 0,
  failed: 1,
  waiting: 1,
};

// Filtering removes the first rows, moves the shared one up and adds the last.
const before = 'First listed row';
const removed = 'Second listed row';
const shared = 'Row in both lists';
const after = 'Row only in the filtered list with a very long name to measure';
const other = 'Row from the other filter';
const lists: Record<string, string[]> = {
  all: [before, removed, shared],
  filtered: [shared, after],
  other: [other],
};
const keys = new Map(
  Object.values(lists)
    .flat()
    .map((target, index) => [target, index + 1]),
);

const scenarios = [
  {
    table: 'Background jobs',
    section: 'jobs',
    link: 'Jobs',
    loading: 'Loading background jobs',
    path: '**/api/v1/admin/jobs*',
    param: 'state',
    values: ['failed', 'queued'],
    answer: (rows: string[]) => ({
      items: rows.map((target) => job(`job-${keys.get(target)}`, target)),
      pinned: [],
      totals: { running: 0, queued: 1, failed: 1, complete: 3 },
    }),
    choose: (page: Page, value: string) =>
      page
        .getByRole('group', { name: 'Job state' })
        .getByRole('button', {
          name: new RegExp(`^${value === 'failed' ? 'Failed' : 'Queued'}`),
        })
        .click(),
    clear: (page: Page) =>
      page
        .getByRole('group', { name: 'Job state' })
        .getByRole('button', { name: /^All/ })
        .click(),
    refresh: (page: Page) =>
      page.evaluate(() => window.dispatchEvent(new Event('online'))),
  },
  {
    table: 'Analyzed episodes',
    section: 'analysis',
    link: 'Episode analysis',
    loading: 'Loading analysis results',
    path: '**/api/v1/admin/segments/analysis*',
    param: 'status',
    values: ['failed', 'waiting'],
    answer: (rows: string[]) => ({
      items: rows.map((title) => episode(`episode-${keys.get(title)}`, title)),
      shows: [{ id: 'show', title: 'Fixture show', ...coverage }],
      totals: coverage,
    }),
    choose: (page: Page, value: string) =>
      page
        .getByRole('group', { name: 'Episode status' })
        .getByRole('button', {
          name: new RegExp(`^${value === 'failed' ? 'Failed' : 'Waiting'}`),
        })
        .click(),
    clear: (page: Page) =>
      page
        .getByRole('group', { name: 'Episode status' })
        .getByRole('button', { name: /^All/ })
        .click(),
    refresh: (page: Page) =>
      page.evaluate(() => window.dispatchEvent(new Event('online'))),
  },
  {
    table: 'Administrative activity',
    section: 'audit',
    link: 'Audit',
    loading: 'Loading administrative activity',
    path: '**/api/v1/admin/audit?*',
    param: 'action',
    values: ['settings.update', 'user.create'],
    answer: (rows: string[]) => ({
      actors: [],
      actions: ['settings.update', 'user.create'],
      items: rows.map((target) => entry(keys.get(target)!, target)),
      next_before: null,
    }),
    choose: (page: Page, value: string) =>
      page.getByLabel('Audit action', { exact: true }).selectOption(value),
    clear: (page: Page) =>
      page.getByLabel('Audit action', { exact: true }).selectOption(''),
    refresh: (page: Page) =>
      page.getByRole('button', { name: 'Refresh', exact: true }).click(),
  },
];
type Scenario = (typeof scenarios)[number];

// `respond` overrides the answer to a read, and returns nothing to leave it to the list.
async function install(
  page: Page,
  scenario: Scenario,
  respond?: (route: Route, value: string | null) => Promise<void> | undefined,
  settingsSection = scenario.section,
) {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection,
  });
  await page.route('**/api/v1/admin/segments', (route) =>
    route.fulfill({ json: { config: { local: true, external: false } } }),
  );
  // The longest setting, so that a busy runner still draws frames of each animation.
  await page.route('**/api/v1/me/appearance', (route) =>
    route.fulfill({ json: { theme: 'light', animation_duration_ms: 500 } }),
  );
  const reads: string[] = [];
  await page.route(scenario.path, (route) => {
    const value = new URL(route.request().url()).searchParams.get(
      scenario.param,
    );
    reads.push(value ?? '');
    const handled = respond?.(route, value);
    if (handled) return handled;
    const list =
      value === scenario.values[0]
        ? lists.filtered
        : value === scenario.values[1]
          ? lists.other
          : lists.all;
    return route.fulfill({ json: scenario.answer(list) });
  });
  await page.goto('/');
  const table = page.getByRole('table', { name: scenario.table });
  return { fixture, reads, table };
}

async function open(
  page: Page,
  scenario: Scenario,
  respond?: Parameters<typeof install>[2],
) {
  const installed = await install(page, scenario, respond);
  await expect(installed.table.getByText(before)).toBeVisible();
  return installed;
}

const watchLayers = (page: Page) =>
  page.addInitScript(() => {
    new MutationObserver(() => {
      if (document.querySelector('[data-motion-layer]'))
        document.documentElement.dataset.layerSeen = 'true';
    }).observe(document, { childList: true, subtree: true });
  });
const layerSeen = (page: Page) =>
  page.evaluate(() => document.documentElement.dataset.layerSeen);

// The app uses the system font, and these are about as wide as they come.
const wideFont = '*{font-family:Verdana,"DejaVu Sans",sans-serif !important}';

const motionRoot = (table: ReturnType<Page['getByRole']>) =>
  table.locator('xpath=ancestor::*[@aria-busy][1]');

type Recorded = {
  moved: number;
  ghosts: number;
  skeletons: number;
  tinted: number;
  resized: number;
};
type Recording = { motionRecording: Recorded };

// Samples every frame, since a fast read can be over between two checks.
const recordMotion = (page: Page) =>
  page.evaluate(() => {
    const seen = { moved: 0, ghosts: 0, skeletons: 0, tinted: 0, resized: 0 };
    (window as unknown as Recording).motionRecording = seen;
    const sample = () => {
      const running = document.getAnimations().flatMap((animation) => {
        const effect = animation.effect as KeyframeEffect | null;
        return effect ? [{ effect, frames: effect.getKeyframes() }] : [];
      });
      seen.ghosts = Math.max(
        seen.ghosts,
        document.querySelectorAll('[data-motion-layer] > *').length,
      );
      seen.moved = Math.max(
        seen.moved,
        running.filter(
          ({ effect, frames }) =>
            effect.target?.matches('[data-motion-row]') &&
            frames.some((frame) => 'transform' in frame),
        ).length,
      );
      seen.tinted = Math.max(
        seen.tinted,
        running.filter(({ frames }) =>
          frames.some((frame) =>
            String(frame.backgroundColor).includes('color-mix'),
          ),
        ).length,
      );
      seen.resized = Math.max(
        seen.resized,
        running.filter(({ frames }) =>
          frames.some((frame) => 'height' in frame),
        ).length,
      );
      if (document.querySelector('[data-skeleton]')) seen.skeletons++;
      requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  });
const recordedMotion = (page: Page) =>
  page.evaluate(() => (window as unknown as Recording).motionRecording);

for (const scenario of scenarios) {
  test(`${scenario.table} keep their rows until a new filter answers`, async ({
    page,
  }) => {
    const gate = responseGate();
    const { fixture, table } = await open(page, scenario, (route, value) =>
      value === scenario.values[0]
        ? gate.promise.then(() =>
            route.fulfill({ json: scenario.answer(lists.filtered) }),
          )
        : undefined,
    );
    await scenario.choose(page, scenario.values[0]);
    await expect(motionRoot(table)).toHaveAttribute('aria-busy', 'true');
    await expect(
      motionRoot(table).locator('[data-motion-indicator]'),
    ).toHaveCount(1);
    await expect(table.getByText(before)).toBeVisible();
    await expect(page.locator('[data-skeleton]')).toHaveCount(0);
    gate.release();
    await expect(table.getByText(after)).toBeVisible();
    await expect(motionRoot(table)).toHaveAttribute('aria-busy', 'false');
    await expect(
      motionRoot(table).locator('[data-motion-indicator]'),
    ).toHaveCount(0);
    await expect(table.getByText(before)).toHaveCount(0);
    expect(fixture.errors).toEqual([]);
  });

  test(`${scenario.table} cancel a read a newer filter replaced`, async ({
    page,
  }) => {
    let held: Route | undefined;
    const { fixture, reads, table } = await open(
      page,
      scenario,
      (route, value) => {
        if (value !== scenario.values[0]) return;
        held = route;
        return Promise.resolve();
      },
    );
    const cancelled = page.waitForEvent(
      'requestfailed',
      (request) =>
        new URL(request.url()).searchParams.get(scenario.param) ===
        scenario.values[0],
    );
    await scenario.choose(page, scenario.values[0]);
    await expect.poll(() => Boolean(held)).toBe(true);
    await scenario.choose(page, scenario.values[1]);
    // It reads at once, while the first read has still not answered.
    await expect.poll(() => reads.at(-1)).toBe(scenario.values[1]);
    await cancelled;
    await expect(table.getByText(other)).toBeVisible();
    await expect(table.getByText(before)).toHaveCount(0);
    expect(fixture.errors).toEqual([]);
  });

  test(`${scenario.table} glide to a new filter and clean up after`, async ({
    page,
  }) => {
    const { fixture, table } = await open(page, scenario);
    const columns = () =>
      table
        .locator('thead th')
        .evaluateAll((cells) =>
          cells.map((cell) => Math.round(cell.getBoundingClientRect().left)),
        );
    const pinned = await columns();
    await recordMotion(page);
    await scenario.choose(page, scenario.values[0]);
    await expect(table.getByText(after)).toBeVisible();
    await expect(page.locator('[data-motion-layer]')).toHaveCount(0);
    const seen = await recordedMotion(page);
    expect(seen.moved).toBeGreaterThan(0);
    expect(seen.ghosts).toBeGreaterThan(0);
    expect(seen.skeletons).toBe(0);
    expect(await columns()).toEqual(pinned);
    await expect(motionRoot(table)).toHaveAttribute('aria-busy', 'false');
    expect(await motionRoot(table).getAttribute('style')).toBeFalsy();
    expect(fixture.errors).toEqual([]);
  });

  test(`${scenario.table} do not animate under reduced motion`, async ({
    page,
  }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    const { fixture, table } = await open(page, scenario);
    await page.evaluate(() => {
      const observer = new MutationObserver(() => {
        if (document.querySelector('[data-motion-layer]'))
          document.documentElement.dataset.ghostSeen = 'true';
      });
      observer.observe(document.body, { childList: true, subtree: true });
    });
    await scenario.choose(page, scenario.values[0]);
    await expect(table.getByText(after)).toBeVisible();
    await expect(table.getByText(before)).toHaveCount(0);
    expect(
      await page.evaluate(() => document.documentElement.dataset.ghostSeen),
    ).toBeUndefined();
    expect(fixture.errors).toEqual([]);
  });

  test(`${scenario.table} drop rows a failed filter cannot answer for, and keep them through a failed refresh`, async ({
    page,
  }) => {
    let failing = false;
    const failure = (route: Route) =>
      route.fulfill({
        status: 503,
        json: { error: { code: 'down', message: 'Server unavailable' } },
      });
    const { table } = await open(page, scenario, (route) =>
      failing ? failure(route) : undefined,
    );
    failing = true;
    await scenario.refresh(page);
    await expect(page.getByRole('alert')).toContainText('Server unavailable');
    await expect(table.getByText(before)).toBeVisible();
    await scenario.choose(page, scenario.values[0]);
    await expect(table.getByText(before)).toHaveCount(0);
    await expect(page.getByRole('alert')).toContainText('Server unavailable');
    await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
    failing = false;
    await scenario.choose(page, scenario.values[1]);
    await expect(table.getByText(other)).toBeVisible();
    await expect(page.getByRole('alert')).toHaveCount(0);
  });

  test(`${scenario.table} keep rows through a failed read of the filter they answer`, async ({
    page,
  }) => {
    let failing = false;
    let held = false;
    const { table } = await open(page, scenario, (route, value) => {
      if (value === scenario.values[0]) {
        held = true;
        return Promise.resolve();
      }
      if (failing)
        return route.fulfill({
          status: 503,
          json: { error: { code: 'down', message: 'Server unavailable' } },
        });
    });
    await scenario.choose(page, scenario.values[0]);
    await expect.poll(() => held).toBe(true);
    failing = true;
    await scenario.clear(page);
    await expect(page.getByRole('alert')).toContainText('Server unavailable');
    await expect(table.getByText(before)).toBeVisible();
    await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  });

  test(`${scenario.table} appear without animating when their page waits for them`, async ({
    page,
  }) => {
    await watchLayers(page);
    const gate = responseGate();
    const { fixture, table } = await install(
      page,
      scenario,
      (route) =>
        gate.promise.then(() =>
          route.fulfill({ json: scenario.answer(lists.all) }),
        ),
      'account',
    );
    await page
      .getByRole('navigation', { name: 'Settings navigation' })
      .getByRole('link', { name: scenario.link, exact: true })
      .click();
    // The hidden page has reached its skeleton, which is not on screen.
    await expect(
      page.locator(`[aria-label="${scenario.loading}"]`),
    ).toBeAttached();
    gate.release();
    await expect(table.getByText(before)).toBeVisible();
    expect(await layerSeen(page)).toBeUndefined();
    expect(fixture.errors).toEqual([]);
  });

  test(`${scenario.table} crossfade from a skeleton on screen`, async ({
    page,
  }) => {
    await watchLayers(page);
    const gate = responseGate();
    const opening = open(page, scenario, (route) =>
      gate.promise.then(() =>
        route.fulfill({ json: scenario.answer(lists.all) }),
      ),
    );
    await expect(
      page.locator(`[aria-label="${scenario.loading}"]`),
    ).toBeVisible();
    gate.release();
    const { fixture } = await opening;
    expect(await layerSeen(page)).toBe('true');
    await expect(page.locator('[data-motion-layer]')).toHaveCount(0);
    expect(fixture.errors).toEqual([]);
  });

  test(`${scenario.table} stack once their columns stop fitting`, async ({
    page,
  }) => {
    const { fixture, table } = await open(page, scenario);
    await expect(table.locator('thead')).toBeVisible();
    for (const font of [null, wideFont]) {
      if (font) await page.addStyleTag({ content: font });
      for (const width of [1000, 900, 840, 800, 721, 720, 390, 320]) {
        await page.setViewportSize({ width, height: 900 });
        await expect(table.locator('thead')).toBeHidden();
        // Every table on the page, since Episode analysis has a second one.
        expect(
          await page.evaluate(() =>
            [
              ...document.querySelectorAll(
                '.settings-card td, .settings-card th',
              ),
            ].flatMap((cell) => {
              const past =
                cell.getBoundingClientRect().right -
                cell.closest('.settings-card')!.getBoundingClientRect().right;
              return past > 1 ? [cell.textContent] : [];
            }),
          ),
          `${font ?? 'default font'} at ${width} px`,
        ).toEqual([]);
        expect(
          await page.evaluate(
            () =>
              document.documentElement.scrollWidth <=
              document.documentElement.clientWidth,
          ),
        ).toBe(true);
      }
    }
    expect(fixture.errors).toEqual([]);
  });
}

// How opaque a row starts out in the fade it is in, if it is in one.
const fadeStart = (row: ReturnType<Page['locator']>) =>
  row.evaluate((element) => {
    for (const animation of element.getAnimations()) {
      const first = (animation.effect as KeyframeEffect).getKeyframes()[0];
      if ('opacity' in first) return Number(first.opacity);
    }
    return null;
  });

// How opaque the leaving copy of a row starts out, if it has one.
const leavingOpacity = (page: Page, text: string) =>
  page.evaluate((text) => {
    for (const animation of document.getAnimations()) {
      const effect = animation.effect as KeyframeEffect;
      const first = effect.getKeyframes()[0];
      if (
        'opacity' in first &&
        effect.target?.closest('[data-motion-layer]') &&
        effect.target.textContent?.includes(text)
      )
        return Number(first.opacity);
    }
    return null;
  }, text);

const manyJobs = (count: number) => ({
  items: Array.from({ length: count }, (_, index) =>
    job(`job-${index}`, `Row ${index}`),
  ),
  pinned: [],
  totals: { running: 0, queued: 0, failed: 2, complete: 100 },
});

async function scrolledJobs(page: Page, filtered: number) {
  const gate = responseGate();
  const { table } = await install(page, scenarios[0], (route, value) =>
    value
      ? gate.promise.then(() => route.fulfill({ json: manyJobs(filtered) }))
      : route.fulfill({ json: manyJobs(100) }),
  );
  await expect(table.getByText('Row 50')).toBeVisible();
  const scroller = page.locator('.settings-scroll');
  await scroller.evaluate((element) => (element.scrollTop = 97));
  const bar = page.getByRole('group', { name: 'Job state' });
  const barTop = async () =>
    (await bar.evaluate((element) => element.getBoundingClientRect().top)) | 0;
  return { gate, table, scroller, barTop };
}

test('the filter bar stays put while a scrolled list changes and stays long', async ({
  page,
}) => {
  const { gate, table, barTop } = await scrolledJobs(page, 40);
  const top = await barTop();
  await scenarios[0].choose(page, scenarios[0].values[0]);
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(1);
  expect(await barTop()).toBe(top);
  gate.release();
  await expect(table.getByText('Row 50')).toHaveCount(0);
  expect(await barTop()).toBe(top);
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  expect(await barTop()).toBe(top);
});

test('a list too short to stay scrolled brings the page back gradually', async ({
  page,
}) => {
  const { gate, table, scroller, barTop } = await scrolledJobs(page, 2);
  const top = await barTop();
  await pauseAnimations(page);
  await scenarios[0].choose(page, scenarios[0].values[0]);
  gate.release();
  await expect(table.getByText('Row 0', { exact: true })).toBeVisible();
  await expect(table.getByText('Row 2', { exact: true })).toHaveCount(0);
  await page.waitForFunction(() =>
    document
      .getAnimations()
      .some((animation) => animation.playState === 'paused'),
  );
  const tops: number[] = [await barTop()];
  expect(tops[0]).toBe(top);
  for (let step = 1; step <= 40; step++) {
    await page.evaluate((progress) => {
      for (const animation of document.getAnimations()) {
        if (animation.playState !== 'paused') continue;
        const { duration } = animation.effect!.getTiming();
        animation.currentTime = Number(duration) * progress;
      }
    }, step / 40);
    tops.push(await barTop());
  }
  expect(await scroller.evaluate((element) => element.scrollTop)).toBe(0);
  const end = tops[tops.length - 1];
  expect(end).toBeGreaterThan(top);
  const steps = tops.slice(1).map((value, index) => value - tops[index]);
  expect(Math.max(...steps)).toBeLessThan(end - top);
  expect(Math.min(...steps)).toBeGreaterThanOrEqual(0);
});

test('a show that arrives during a refresh moves the page gradually', async ({
  page,
}) => {
  const scenario = scenarios[1];
  const { fixture, table } = await open(page, scenario);
  await recordMotion(page);
  await page.route(scenario.path, (route) =>
    route.fulfill({
      json: {
        ...scenario.answer(lists.all),
        shows: [
          { id: 'show', title: 'Fixture show', ...coverage },
          { id: 'new-show', title: 'New show', ...coverage },
        ],
      },
    }),
  );
  await scenario.refresh(page);
  await expect(
    page.getByRole('table', { name: 'Coverage by show' }).getByText('New show'),
  ).toBeVisible();
  await expect(table.getByText(before)).toBeVisible();
  await expect
    .poll(async () => (await recordedMotion(page)).tinted)
    .toBeGreaterThan(0);
  expect((await recordedMotion(page)).resized).toBeGreaterThan(0);
  expect(fixture.errors).toEqual([]);
});

test('a selected row keeps its colour while it moves', async ({ page }) => {
  const scenario = scenarios[1];
  const show = (id: string, title: string) => ({ id, title, ...coverage });
  const { fixture } = await open(page, scenario);
  const shows = page.getByRole('table', { name: 'Coverage by show' });
  await page.route(scenario.path, (route) =>
    route.fulfill({
      json: {
        ...scenario.answer(lists.all),
        shows: [show('other', 'Other show'), show('show', 'Fixture show')],
      },
    }),
  );
  await scenario.refresh(page);
  const selected = shows.getByRole('button', { name: 'Fixture show' });
  await selected.click();
  await expect(selected).toHaveAttribute('aria-pressed', 'true');
  // Its own colour transition has to finish for the colour to be its final one.
  await expect
    .poll(() =>
      page
        .locator('[data-motion-row="show:show"]')
        .evaluate((row) => row.getAnimations().length),
    )
    .toBe(0);
  await page.route(scenario.path, (route) =>
    route.fulfill({
      json: {
        ...scenario.answer(lists.all),
        shows: [
          show('new-show', 'New show'),
          show('other', 'Other show'),
          show('show', 'Fixture show'),
        ],
      },
    }),
  );
  await pauseAnimations(page);
  await scenario.refresh(page);
  const paint = (key: string) =>
    page.locator(`[data-motion-row="show:${key}"]`).evaluate((element) => {
      for (const animation of element.getAnimations()) {
        const [first] = (animation.effect as KeyframeEffect).getKeyframes();
        if ('transform' in first) return String(first.backgroundColor);
      }
      return null;
    });
  await expect.poll(() => paint('other')).not.toBeNull();
  await expect.poll(() => paint('show')).not.toBeNull();
  expect(await paint('show')).not.toBe(await paint('other'));
  expect(fixture.errors).toEqual([]);
});

test('coverage keeps its columns until they stop fitting', async ({ page }) => {
  const { fixture } = await open(page, scenarios[1]);
  const shows = page.getByRole('table', { name: 'Coverage by show' });
  for (const font of [null, wideFont]) {
    if (font) await page.addStyleTag({ content: font });
    await page.setViewportSize({ width: 1000, height: 900 });
    await expect(shows.locator('thead')).toBeVisible();
    await page.setViewportSize({ width: 721, height: 900 });
    await expect(shows.locator('thead')).toBeHidden();
    // Stacked, every cell needs its own label, and they line up with the title.
    for (const label of ['Episodes', 'Intro', 'Credits', 'Nothing found'])
      await expect(
        shows.locator('tbody').getByText(label, { exact: true }),
      ).toBeVisible();
    const edges = await page
      .locator('[data-motion-row="show:show"] td')
      .evaluateAll((cells) =>
        cells.map((cell) => cell.getBoundingClientRect().left),
      );
    expect(edges[1]).toBeCloseTo(edges[0], 0);
  }
  expect(fixture.errors).toEqual([]);
});

test('a scheduled deletion that arrives during a refresh is marked', async ({
  page,
}) => {
  const { fixture } = await open(page, scenarios[2]);
  const item = (id: string, title: string) => ({
    id,
    title,
    state: 'pending',
    reason: 'storage_limit',
    due_at: 4_000_000_000,
    error: null,
  });
  let items = [item('first', 'First title')];
  await page.route('**/api/v1/admin/retention/deletions', (route) =>
    route.fulfill({
      json: { items, totals: { pending: items.length, history: 0 } },
    }),
  );
  await page
    .getByLabel('Audit view', { exact: true })
    .selectOption('scheduled');
  const scheduled = page.getByRole('region', { name: 'Scheduled deletions' });
  await expect(scheduled.getByText('First title')).toBeVisible();
  await expect(page.locator('[data-motion-layer]')).toHaveCount(0);
  await recordMotion(page);
  items = [...items, item('second', 'Second title')];
  await page.evaluate(() => window.dispatchEvent(new Event('online')));
  await expect(scheduled.getByText('Second title')).toBeVisible();
  await expect.poll(async () => (await recordedMotion(page)).tinted).toBe(1);
  expect(fixture.errors).toEqual([]);
});

test('rows dimmed by a slow read leave from that dimness', async ({ page }) => {
  const scenario = scenarios[0];
  const gate = responseGate();
  const { table } = await open(page, scenario, (route, value) =>
    value === scenario.values[0]
      ? gate.promise.then(() =>
          route.fulfill({ json: scenario.answer(lists.filtered) }),
        )
      : undefined,
  );
  await pauseAnimations(page);
  await scenario.choose(page, scenario.values[0]);
  await expect(motionRoot(table).locator('> .transition-opacity')).toHaveCSS(
    'opacity',
    '0.55',
  );
  gate.release();
  await expect(table.getByText(after)).toBeVisible();
  await expect.poll(() => leavingOpacity(page, removed)).toBeCloseTo(0.55);
});

test('a view dimmed by a slow read crossfades from that dimness', async ({
  page,
}) => {
  await open(page, scenarios[2]);
  const gate = responseGate();
  await page.route('**/api/v1/admin/retention/deletions', (route) =>
    gate.promise.then(() =>
      route.fulfill({
        json: { items: [], totals: { pending: 0, history: 0 } },
      }),
    ),
  );
  await pauseAnimations(page);
  await page
    .getByLabel('Audit view', { exact: true })
    .selectOption('scheduled');
  await expect(
    page.locator('[aria-busy="true"] > .transition-opacity'),
  ).toHaveCSS('opacity', '0.55');
  gate.release();
  const copy = page.locator('[data-motion-layer] > *');
  await expect(copy).toBeAttached();
  await expect.poll(() => fadeStart(copy)).toBeCloseTo(0.55);
});

test('a row removed while still invisible leaves nothing behind', async ({
  page,
}) => {
  const scenario = scenarios[0];
  const { table } = await open(page, scenario);
  await pauseAnimations(page);
  await scenario.choose(page, scenario.values[0]);
  await expect(table.getByText(after)).toBeVisible();
  await scenario.choose(page, scenario.values[1]);
  await expect(table.getByText(other)).toBeVisible();
  await expect.poll(() => leavingOpacity(page, shared)).toBe(1);
  expect(await leavingOpacity(page, after)).toBeNull();
});

test('a row still fading in carries on from there when it stays listed', async ({
  page,
}) => {
  const scenario = scenarios[0];
  let rows = lists.filtered;
  const { table } = await open(page, scenario, (route, value) =>
    value === scenario.values[0]
      ? route.fulfill({ json: scenario.answer(rows) })
      : undefined,
  );
  await pauseAnimations(page);
  await scenario.choose(page, scenario.values[0]);
  const row = table.locator('tbody').filter({ hasText: after });
  await expect(row).toBeVisible();
  const opacity = await row.evaluate((element) => {
    for (const animation of element.getAnimations())
      if ('opacity' in (animation.effect as KeyframeEffect).getKeyframes()[0])
        animation.currentTime =
          Number(animation.effect!.getTiming().duration) * 0.6;
    return Number(getComputedStyle(element).opacity);
  });
  expect(opacity).toBeGreaterThan(0);
  expect(opacity).toBeLessThan(1);
  rows = [before, ...lists.filtered];
  await scenario.refresh(page);
  await expect(table.getByText(before)).toBeVisible();
  await expect.poll(() => fadeStart(row)).toBeCloseTo(opacity);
});

test('a row coming back while it fades out carries on from where it is', async ({
  page,
}) => {
  const scenario = scenarios[0];
  const { table } = await open(page, scenario);
  await pauseAnimations(page);
  await scenario.choose(page, scenario.values[0]);
  await expect(table.getByText(after)).toBeVisible();
  await expect.poll(() => leavingOpacity(page, before)).toBe(1);
  // Halfway through the part of the animation where it fades.
  await seekAnimations(page, 0.2);
  await scenario.clear(page);
  const row = table.locator('tbody').filter({ hasText: before });
  await expect(row).toBeVisible();
  await expect.poll(() => fadeStart(row)).toBeCloseTo(0.5);
  expect(await leavingOpacity(page, before)).toBeNull();
});

test('a row that stays removed keeps fading through the next update', async ({
  page,
}) => {
  const scenario = scenarios[0];
  const { table } = await open(page, scenario);
  await pauseAnimations(page);
  await scenario.choose(page, scenario.values[0]);
  await expect(table.getByText(after)).toBeVisible();
  await expect.poll(() => leavingOpacity(page, before)).toBe(1);
  await seekAnimations(page, 0.2);
  await scenario.choose(page, scenario.values[1]);
  await expect(table.getByText(other)).toBeVisible();
  await expect.poll(() => leavingOpacity(page, before)).toBeCloseTo(0.5);
});

test('a button disabled during a read waits to dim with reduced motion too', async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const { gate } = await scrolledJobs(page, 40);
  await scenarios[0].choose(page, scenarios[0].values[0]);
  const older = page.getByRole('button', { name: 'Load older jobs' });
  await expect(older).toBeDisabled();
  await expect(older).toHaveCSS('transition-delay', '0.2s');
  gate.release();
});

test('the first show of an empty library animates in', async ({ page }) => {
  const scenario = scenarios[1];
  let shows: unknown[] = [];
  await install(page, scenario, (route) =>
    route.fulfill({ json: { ...scenario.answer([]), shows } }),
  );
  await expect(page.getByText('No episodes to analyze yet.')).toBeVisible();
  await recordMotion(page);
  shows = [{ id: 'show', title: 'First show', ...coverage }];
  await scenario.refresh(page);
  await expect(
    page
      .getByRole('table', { name: 'Coverage by show' })
      .getByText('First show'),
  ).toBeVisible();
  await expect
    .poll(async () => (await recordedMotion(page)).resized)
    .toBeGreaterThan(0);
});

test('a long user name stays inside its column', async ({ page }) => {
  const name = 'alexander.smith.with.a.long.name@example.com';
  const { table } = await open(page, scenarios[2], (route) =>
    route.fulfill({
      json: {
        ...scenarios[2].answer([]),
        items: [{ ...entry(1, before), actor: name }],
      },
    }),
  );
  const cell = table.getByRole('cell', { name });
  await expect(cell.getByText(name)).toHaveAttribute('title', name);
  expect(
    await cell.evaluate(
      (element) => element.scrollWidth <= element.clientWidth,
    ),
  ).toBe(true);
});

test('the audit view waits for its data, and coming back rereads activity', async ({
  page,
}) => {
  const scenario = scenarios[2];
  const { fixture, reads, table } = await open(page, scenario);
  const gate = responseGate();
  await page.route('**/api/v1/admin/retention/deletions', async (route) => {
    await gate.promise;
    await route.fulfill({
      json: {
        items: [
          {
            id: 'scheduled-item',
            title: 'Scheduled title',
            state: 'pending',
            reason: 'storage_limit',
            due_at: 4_000_000_000,
            error: null,
          },
        ],
        totals: { pending: 1, history: 0 },
      },
    });
  });
  const view = page.getByLabel('Audit view', { exact: true });
  const scheduled = page.getByRole('region', { name: 'Scheduled deletions' });
  await view.selectOption('scheduled');
  await expect(view).toHaveValue('scheduled');
  await expect(table).toBeVisible();
  await expect(scheduled).toHaveCount(0);
  await expect(page.locator('[data-skeleton]')).toHaveCount(0);
  gate.release();
  await expect(scheduled.getByText('Scheduled title')).toBeVisible();
  await expect(table).toBeHidden();
  const count = reads.length;
  await view.selectOption('activity');
  await expect(table.getByText(before)).toBeVisible();
  await expect(scheduled).toHaveCount(0);
  await expect.poll(() => reads.length).toBeGreaterThan(count);
  expect(fixture.errors).toEqual([]);
});

test('coming back to activity keeps its rows when the reread fails', async ({
  page,
}) => {
  let failing = false;
  const { table } = await open(page, scenarios[2], (route) =>
    failing
      ? route.fulfill({
          status: 503,
          json: { error: { code: 'down', message: 'Server unavailable' } },
        })
      : undefined,
  );
  await page.route('**/api/v1/admin/retention/deletions', (route) =>
    route.fulfill({ json: { items: [], totals: { pending: 0, history: 0 } } }),
  );
  const view = page.getByLabel('Audit view', { exact: true });
  await view.selectOption('scheduled');
  await expect(
    page.getByRole('region', { name: 'Scheduled deletions' }),
  ).toBeVisible();
  failing = true;
  await view.selectOption('activity');
  await expect(page.getByRole('alert')).toContainText('Server unavailable');
  await expect(table.getByText(before)).toBeVisible();
});

test('going back to the audit view on show ends the wait for another', async ({
  page,
}) => {
  const { fixture, table } = await open(page, scenarios[2]);
  let hold = true;
  await page.route('**/api/v1/admin/retention/deletions', (route) => {
    if (hold) {
      hold = false;
      return;
    }
    return route.fulfill({
      json: { items: [], totals: { pending: 0, history: 0 } },
    });
  });
  const view = page.getByLabel('Audit view', { exact: true });
  const scheduled = page.getByRole('region', { name: 'Scheduled deletions' });
  const waiting = page.locator('[aria-busy="true"]');

  await view.selectOption('scheduled');
  await expect(waiting).toHaveCount(1);
  await view.selectOption('activity');
  await expect(waiting).toHaveCount(0);
  await expect(table).toBeVisible();

  await view.selectOption('scheduled');
  await expect(scheduled).toBeVisible();
  await expect(waiting).toHaveCount(0);
  hold = true;
  await view.selectOption('deletions');
  await expect(waiting).toHaveCount(1);
  await view.selectOption('scheduled');
  await expect(waiting).toHaveCount(0);
  await expect(scheduled).toBeVisible();
  expect(fixture.errors).toEqual([]);
});

test('a row that arrives during a refresh is marked, and one from a filter is not', async ({
  page,
}) => {
  const scenario = scenarios[0];
  const { fixture, table } = await open(page, scenario);
  await recordMotion(page);
  await scenario.choose(page, scenario.values[0]);
  await expect(table.getByText(after)).toBeVisible();
  await expect(page.locator('[data-motion-layer]')).toHaveCount(0);
  expect((await recordedMotion(page)).tinted).toBe(0);
  await page.route(scenario.path, (route) =>
    route.fulfill({ json: scenario.answer([...lists.filtered, before]) }),
  );
  await scenario.refresh(page);
  await expect(table.getByText(before)).toBeVisible();
  await expect.poll(async () => (await recordedMotion(page)).tinted).toBe(1);
  expect(fixture.errors).toEqual([]);
});
