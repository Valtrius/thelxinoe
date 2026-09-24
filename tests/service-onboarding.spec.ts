import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test('saved Prowlarr indexers use the same one-second test feedback as setup', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  let finish!: () => void;
  await page.route(
    '**/api/v1/admin/support/support-prowlarr',
    async (route) => {
      if (route.request().method() !== 'POST') return route.fallback();
      expect(route.request().postDataJSON()).toEqual({
        action: 'test',
        item_id: 1,
      });
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      return route.fulfill({ json: { tested: true } });
    },
  );
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('button', { name: 'Prowlarr', exact: true })
    .click();
  const row = page
    .locator('.service-row')
    .filter({ hasText: 'Fixture indexer' });
  const button = row.getByRole('button', { name: 'Test', exact: true });
  await page.clock.install();
  await page.clock.pauseAt(new Date());
  const bounds = await button.boundingBox();
  await button.click();
  const checking = row.getByRole('button', {
    name: 'Testing connection',
    exact: true,
  });
  await expect(checking).toBeVisible();
  await expect(checking).toBeDisabled();
  expect(await checking.boundingBox()).toEqual(bounds);
  await expect(
    page.getByRole('button', { name: 'Add indexer', exact: true }),
  ).toBeEnabled();
  finish();
  const success = row.getByRole('button', {
    name: 'Connection successful',
    exact: true,
  });
  await expect(success).toBeVisible();
  expect(await success.boundingBox()).toEqual(bounds);
  await testInfo.attach('saved-indexer-test-success', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  await page.clock.runFor(999);
  await expect(success).toBeVisible();
  await page.clock.runFor(1);
  await expect(button).toBeVisible();
  expect(await button.boundingBox()).toEqual(bounds);
  expect(fixture.errors).toEqual([]);
});

for (const reducedMotion of ['no-preference', 'reduce'] as const) {
  test(`quality drag opens an animated drop space (${reducedMotion})`, async ({
    page,
  }, testInfo) => {
    await page.emulateMedia({ reducedMotion });
    const fixture = await installUiFixture(page, {
      role: 'admin',
      settingsSection: 'services',
    });
    const qualities = [
      { id: 1, name: 'WEBDL-1080p', resolution: 1080 },
      { id: 2, name: 'Bluray-1080p', resolution: 1080 },
      { id: 3, name: 'WEBDL-2160p', resolution: 2160 },
      { id: 4, name: 'Remux-2160p', resolution: 2160 },
    ];
    await page.route(
      '**/api/v1/admin/managers/manager-radarr/quality-profiles',
      (route) => route.fulfill({ json: { qualities } }),
    );
    await page.goto('/');
    await page
      .getByRole('navigation', { name: 'Select service' })
      .getByRole('button', { name: 'Radarr', exact: true })
      .click();
    await page.getByRole('button', { name: 'Create quality profile' }).click();
    const dialog = page.getByRole('dialog');
    const list = dialog.getByRole('list', { name: 'Quality priority' });
    const first = list.locator('[data-item="4"]');
    const grip = list.getByRole('button', {
      name: 'Reorder WEBDL-1080p',
      exact: true,
    });
    const originalFirst = (await first.boundingBox())!,
      start = (await grip.boundingBox())!;
    const x = start.x + start.width / 2,
      y = originalFirst.y + 3;
    // Record actual intermediate frames; a final screenshot cannot detect a jump.
    await first.evaluate((node) => {
      const samples: number[] = [];
      (window as unknown as { dragFrames: number[] }).dragFrames = samples;
      const sample = () => {
        samples.push(node.getBoundingClientRect().y);
        if (samples.length < 120) requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    });
    await page.mouse.move(x, start.y + start.height / 2);
    await page.mouse.down();
    await page.mouse.move(x + 12, y, { steps: 1 });
    const ghost = list.locator('[data-drop-placeholder]');
    const preview = dialog.locator('[data-drag-preview]');
    await expect(ghost).toBeVisible();
    await expect(preview).toBeVisible();
    await expect(ghost).toContainText('WEBDL-1080p');
    await expect(preview).toContainText('WEBDL-1080p');
    await expect
      .poll(async () =>
        Math.abs((await ghost.boundingBox())!.y - originalFirst.y),
      )
      .toBeLessThanOrEqual(1);
    expect((await ghost.boundingBox())!.height).toBeGreaterThanOrEqual(40);
    await expect
      .poll(async () => (await first.boundingBox())!.y - originalFirst.y)
      .toBeGreaterThan(40);
    const finalY = (await first.boundingBox())!.y;
    if (reducedMotion === 'no-preference') {
      const frames = await page.evaluate(
        () => (window as unknown as { dragFrames: number[] }).dragFrames,
      );
      expect(
        frames.some(
          (value) => value > originalFirst.y + 1 && value < finalY - 1,
        ),
      ).toBe(true);
      await testInfo.attach('drag-animation-frames', {
        body: JSON.stringify(frames),
        contentType: 'application/json',
      });
    } else {
      expect(await first.evaluate((node) => node.getAnimations().length)).toBe(
        0,
      );
    }
    await testInfo.attach(`quality-drop-placeholder-${reducedMotion}`, {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    // Reverse direction without dropping: the gap and animated rows must follow.
    const lower = (await list.boundingBox())!;
    await page.mouse.move(x, lower.y + lower.height - 2, { steps: 1 });
    await expect
      .poll(async () =>
        Math.abs((await first.boundingBox())!.y - originalFirst.y),
      )
      .toBeLessThanOrEqual(1);
    await page.mouse.move(x, y, { steps: 1 });
    await expect
      .poll(async () =>
        Math.abs((await ghost.boundingBox())!.y - originalFirst.y),
      )
      .toBeLessThanOrEqual(1);
    await page.mouse.up();
    await expect(ghost).toHaveCount(0);
    await expect(preview).toHaveCount(0);
    await expect(list.getByRole('listitem').first()).toHaveText('WEBDL-1080p');
    await expect(grip).toBeFocused();
    expect(fixture.errors).toEqual([]);
  });
}

test('indexer test success briefly replaces the button without moving the form', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  let finish!: () => void;
  let invalid = true;
  await page.route(
    '**/api/v1/admin/support/support-prowlarr/indexers/**',
    async (route) => {
      if (route.request().url().endsWith('/schema'))
        return route.fulfill({
          json: {
            profiles: [{ id: 1, name: 'Standard' }],
            items: [
              {
                name: 'DrunkenSlug',
                implementation: 'Newznab',
                protocol: 'usenet',
                privacy: 'private',
                fields: [
                  { name: 'apiKey', label: 'API Key', type: 'password' },
                ],
              },
            ],
          },
        });
      if (invalid)
        return route.fulfill({
          status: 409,
          json: {
            error: {
              message:
                'Invalid API key. Check your indexer API key and try again.',
            },
          },
        });
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      await route.fulfill({ json: { tested: true } });
    },
  );
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('button', { name: 'Prowlarr', exact: true })
    .click();
  await page.getByRole('button', { name: 'Add indexer', exact: true }).click();
  await page.getByRole('button', { name: 'DrunkenSlug' }).click();
  const dialog = page.getByRole('dialog');
  const input = dialog.getByLabel('API Key', { exact: true });
  const testButton = dialog.getByRole('button', {
    name: 'Test connection',
    exact: true,
  });
  await input.fill('invalid-fixture-key');
  await testButton.click();
  await expect(dialog.getByRole('alert')).toHaveText(
    'Invalid API key. Check your indexer API key and try again.',
  );
  await expect(input).toHaveValue('invalid-fixture-key');
  invalid = false;
  await input.fill('valid-fixture-key');
  await page.clock.install();
  await page.clock.pauseAt(new Date());
  await testButton.click();
  const busy = dialog.getByRole('button', {
    name: 'Testing connection',
    exact: true,
  });
  await expect(busy).toBeVisible();
  const bounds = await busy.boundingBox();
  const add = dialog.getByRole('button', { name: 'Add indexer', exact: true });
  const addBounds = await add.boundingBox();
  finish();
  const success = dialog.getByRole('button', {
    name: 'Connection successful',
    exact: true,
  });
  await expect(success).toBeVisible();
  expect(await success.boundingBox()).toEqual(bounds);
  expect(await add.boundingBox()).toEqual(addBounds);
  await testInfo.attach('indexer-test-success', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  await page.clock.runFor(999);
  await expect(success).toBeVisible();
  await page.clock.runFor(1);
  await expect(testButton).toBeVisible();
  expect(await testButton.boundingBox()).toEqual(bounds);
  expect(fixture.errors).toEqual([]);
});

// Browser coverage for settings embedded in the services workspace: modal
// containment, overlapping controls, optional limits, and persisted selections.
// Upstream APIs are fixtures; this does not claim a paid indexer connection.
for (const width of [1440, 390]) {
  test(`indexer setup preserves optional settings in a usable modal at ${width}px`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, {
      role: 'admin',
      settingsSection: 'services',
    });
    await page.setViewportSize({ width, height: 900 });
    let added = false;
    const drafts: Record<string, unknown>[] = [];
    await page.route(
      '**/api/v1/admin/support/support-prowlarr/indexers**',
      async (route) => {
        if (route.request().url().endsWith('/schema'))
          return route.fulfill({
            json: {
              profiles: [{ id: 1, name: 'Standard' }],
              items: ['DrunkenSlug', 'NZBgeek', 'Torrent fixture'].map(
                (name, index) => ({
                  name,
                  implementation: index === 2 ? 'Cardigann' : 'Newznab',
                  protocol: index === 2 ? 'torrent' : 'usenet',
                  privacy: 'private',
                  fields: [
                    { name: 'apiKey', label: 'API Key', type: 'password' },
                    {
                      name: 'baseSettings.queryLimit',
                      label: 'Query Limit',
                      type: 'number',
                      advanced: true,
                    },
                    {
                      name: 'baseSettings.grabLimit',
                      label: 'Grab Limit',
                      type: 'number',
                      advanced: true,
                    },
                    {
                      name: 'torrentBaseSettings.seedRatio',
                      label: 'Seed Ratio',
                      type: 'number',
                      isFloat: true,
                      advanced: true,
                    },
                    {
                      name: 'baseSettings.limitsUnit',
                      label: 'Limits Unit',
                      type: 'select',
                      value: 0,
                      advanced: true,
                      selectOptions: [{ value: 0, name: 'Day' }],
                    },
                  ],
                }),
              ),
            },
          });
        const draft = route.request().postDataJSON();
        drafts.push(draft.fields);
        if (
          ['baseSettings.queryLimit', 'baseSettings.grabLimit'].some(
            (name) => draft.fields[name] === 0,
          )
        )
          return route.fulfill({
            status: 409,
            json: {
              error: {
                message:
                  'Check these indexer settings: BaseSettings.QueryLimit, BaseSettings.GrabLimit',
              },
            },
          });
        added = !route.request().url().endsWith('/test');
        return route.fulfill({
          json: { tested: !added, id: added ? 2 : null },
        });
      },
    );
    await page.goto('/');
    await page
      .getByRole('navigation', { name: 'Select service' })
      .getByRole('button', { name: 'Prowlarr', exact: true })
      .click();
    const trigger = page.getByRole('button', {
      name: 'Add indexer',
      exact: true,
    });
    await trigger.click();
    const dialog = page.getByRole('dialog', {
      name: 'Add indexer',
      exact: true,
    });
    await expect(dialog).toBeVisible();
    expect(await dialog.evaluate((node) => node.matches(':modal'))).toBe(true);
    const search = dialog.getByRole('textbox', { name: 'Find an indexer' });
    const protocol = dialog.getByRole('combobox', { name: 'Indexer protocol' });
    const a = (await search.boundingBox())!,
      b = (await protocol.boundingBox())!;
    expect(
      a.x + a.width <= b.x ||
        b.x + b.width <= a.x ||
        a.y + a.height <= b.y ||
        b.y + b.height <= a.y,
    ).toBe(true);
    await testInfo.attach(`indexer-catalog-${width}`, {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    await search.fill('drunken');
    await protocol.selectOption('torrent');
    await expect(
      dialog.getByText('No indexers match your search.'),
    ).toBeVisible();
    await protocol.selectOption('usenet');
    await dialog.getByRole('button', { name: 'DrunkenSlug' }).click();
    const setup = page.getByRole('dialog');
    await setup.getByLabel('API Key', { exact: true }).fill('fixture-key');
    await setup.getByRole('button', { name: 'Test connection' }).click();
    await expect(
      setup.getByRole('button', { name: 'Connection successful', exact: true }),
    ).toBeVisible();
    await setup.getByText('Advanced settings', { exact: true }).click();
    await expect(setup.getByLabel('Query Limit', { exact: true })).toHaveValue(
      '',
    );
    await expect(setup.getByLabel('Grab Limit', { exact: true })).toHaveValue(
      '',
    );
    await setup.getByLabel('Query Limit', { exact: true }).fill('100');
    await setup.getByLabel('Seed Ratio', { exact: true }).fill('1.5');
    await setup.getByRole('button', { name: 'Test connection' }).click();
    await expect
      .poll(() => drafts.at(-1)?.['baseSettings.queryLimit'])
      .toBe(100);
    await setup.getByLabel('Query Limit', { exact: true }).fill('');
    await setup
      .getByRole('button', { name: 'Add indexer', exact: true })
      .click();
    await expect.poll(() => added).toBe(true);
    await expect(setup).toHaveCount(0);
    expect(drafts.at(-1)?.['baseSettings.queryLimit']).toBeNull();
    expect(drafts.at(-1)?.['baseSettings.limitsUnit']).toBe(0);
    expect(drafts.at(-1)?.['torrentBaseSettings.seedRatio']).toBe(1.5);
    await trigger.click();
    await search.fill('NZBgeek');
    await dialog.getByRole('button', { name: 'NZBgeek' }).click();
    await setup.getByRole('button', { name: 'Test connection' }).click();
    await expect(
      setup.getByRole('button', { name: 'Connection successful', exact: true }),
    ).toBeVisible();
    await setup.getByRole('button', { name: 'Choose another indexer' }).click();
    await search.fill('Torrent');
    await protocol.selectOption('torrent');
    await dialog.getByRole('button', { name: 'Torrent fixture' }).click();
    await setup.getByRole('button', { name: 'Test connection' }).click();
    await expect(
      setup.getByRole('button', { name: 'Connection successful', exact: true }),
    ).toBeVisible();
    await testInfo.attach(`indexer-${width}`, {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    await page.keyboard.press('Escape');
    await expect(setup).toHaveCount(0);
    await expect(trigger).toBeFocused();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

for (const kind of ['radarr', 'sonarr', 'lidarr']) {
  test(`${kind} creates and selects a quality profile through a modal`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, {
      role: 'admin',
      settingsSection: 'services',
    });
    const label = kind[0].toUpperCase() + kind.slice(1);
    let created: {
      name: string;
      qualities: number[];
      cutoff: number;
      upgrades: boolean;
    } | null = null;
    const qualities =
      kind === 'lidarr'
        ? [
            { id: 4, name: 'MP3-320' },
            { id: 6, name: 'FLAC' },
          ]
        : [
            { id: 1, name: 'SDTV', resolution: 480 },
            { id: 3, name: 'WEBDL-1080p', resolution: 1080 },
            { id: 4, name: 'WEBDL-2160p', resolution: 2160 },
          ];
    await page.route('**/api/v1/admin/managers**', async (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path.endsWith('/managers'))
        return route.fulfill({
          json: {
            items: [
              {
                id: `manager-${kind}`,
                name: label,
                kind,
                container_id: `container-${kind}`,
                port: 7878,
                version: '1.0.0',
                error: null,
                defaults: {
                  root_folder: '/media/test',
                  quality_profile: created ? 2 : 1,
                  metadata_profile: kind === 'lidarr' ? 1 : null,
                  monitored: true,
                },
              },
            ],
          },
        });
      if (path.endsWith('/options'))
        return route.fulfill({
          json: {
            defaults: {
              root_folder: '/media/test',
              quality_profile: created ? 2 : 1,
              metadata_profile: kind === 'lidarr' ? 1 : null,
              monitored: true,
            },
            roots: [{ id: 1, path: '/media/test' }],
            profiles: [
              { id: 1, name: 'Existing' },
              ...(created ? [{ id: 2, name: created.name }] : []),
            ],
            metadata_profiles: [{ id: 1, name: 'Standard' }],
          },
        });
      if (path.endsWith('/quality-profiles')) {
        if (route.request().method() === 'POST') {
          created = route.request().postDataJSON();
          return route.fulfill({ json: { id: 2, name: created!.name } });
        }
        return route.fulfill({ json: { qualities } });
      }
      return route.fallback();
    });
    if (kind === 'lidarr')
      await page.setViewportSize({ width: 390, height: 900 });
    await page.goto('/');
    await page
      .getByRole('navigation', { name: 'Select service' })
      .getByRole('button', { name: label, exact: true })
      .click();
    const trigger = page.getByRole('button', {
      name: 'Create quality profile',
    });
    await trigger.click();
    const dialog = page.getByRole('dialog', { name: 'New quality profile' });
    await expect(dialog).toBeVisible();
    await dialog
      .getByRole('textbox', { name: 'Name', exact: true })
      .fill(`${label} custom`);
    const list = dialog.getByRole('list', { name: 'Quality priority' });
    const rows = list.getByRole('listitem');
    await expect(rows).toHaveText(qualities.toReversed().map((q) => q.name));
    await testInfo.attach(`${kind}-default-priority`, {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    await expect(
      dialog.getByText('Allowed qualities', { exact: true }),
    ).toHaveCount(0);
    if (kind === 'lidarr') {
      await list.getByText('MP3-320', { exact: true }).click();
      await list.getByText('FLAC', { exact: true }).click();
    }
    // Toggling a quality must not append it to the priority list.
    const best = qualities.at(-1)!;
    await list.getByText(best.name, { exact: true }).click();
    await list.getByText(best.name, { exact: true }).click();
    await expect(rows).toHaveText(qualities.toReversed().map((q) => q.name));
    const grip = list.getByRole('button', {
      name: `Reorder ${qualities[0].name}`,
    });
    const from = (await grip.boundingBox())!;
    const to = (await rows.first().boundingBox())!;
    if (kind === 'lidarr') {
      const cdp = await page.context().newCDPSession(page);
      await cdp.send('Input.dispatchTouchEvent', {
        type: 'touchStart',
        touchPoints: [
          { x: from.x + from.width / 2, y: from.y + from.height / 2 },
        ],
      });
      await cdp.send('Input.dispatchTouchEvent', {
        type: 'touchMove',
        touchPoints: [{ x: from.x + from.width / 2, y: to.y + 2 }],
      });
      await cdp.send('Input.dispatchTouchEvent', {
        type: 'touchEnd',
        touchPoints: [],
      });
      await cdp.detach();
    } else {
      await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
      await page.mouse.down();
      await page.mouse.move(from.x + from.width / 2, to.y + 2, { steps: 8 });
      await page.mouse.up();
      await list.getByText('SDTV', { exact: true }).click();
    }
    const displayOrder = [qualities[0], ...qualities.slice(1).reverse()];
    await expect(rows).toHaveText(displayOrder.map((q) => q.name));
    await expect(
      list.getByRole('switch', { name: qualities[0].name, exact: true }),
    ).toBeChecked();
    await dialog
      .getByRole('combobox', { name: 'Upgrade until' })
      .selectOption(String(qualities.at(-1)!.id));
    await testInfo.attach(`${kind}-profile`, {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    await dialog
      .getByRole('button', { name: 'Create and use profile' })
      .click();
    await expect(dialog).toHaveCount(0);
    await expect(
      page.getByRole('combobox', { name: 'Quality profile', exact: true }),
    ).toHaveValue('2');
    expect(created).toMatchObject({
      name: `${label} custom`,
      qualities: [...qualities.slice(1), qualities[0]].map((q) => q.id),
      cutoff: qualities.at(-1)!.id,
      upgrades: true,
    });
    await trigger.click();
    await expect(
      dialog.getByRole('textbox', { name: 'Name', exact: true }),
    ).toHaveValue('');
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await expect(trigger).toBeFocused();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('quality dragging cancels with Escape and scrolls a long list to its drop target', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  const qualities = Array.from({ length: 24 }, (_, i) => ({
    id: i + 1,
    name: `Quality ${i + 1}`,
    resolution: 1080,
  }));
  await page.route(
    '**/api/v1/admin/managers/manager-radarr/quality-profiles',
    (route) => route.fulfill({ json: { qualities } }),
  );
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('button', { name: 'Radarr', exact: true })
    .click();
  await page.getByRole('button', { name: 'Create quality profile' }).click();
  const dialog = page.getByRole('dialog');
  const list = dialog.getByRole('list', { name: 'Quality priority' });
  const rows = list.getByRole('listitem');
  const grip = list.getByRole('button', {
    name: 'Reorder Quality 1',
    exact: true,
  });
  for (const cancel of [true, false]) {
    await grip.scrollIntoViewIfNeeded();
    const from = (await grip.boundingBox())!,
      bounds = (await list.boundingBox())!;
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(from.x + from.width / 2, bounds.y + 2, { steps: 5 });
    if (cancel) {
      await page.keyboard.press('Escape');
      await page.mouse.up();
      await expect(dialog).toBeVisible();
      await expect(rows).toHaveText(
        [...qualities].reverse().map((q) => q.name),
      );
    } else {
      await expect
        .poll(() => list.evaluate((node) => node.scrollTop), { timeout: 10000 })
        .toBe(0);
      await testInfo.attach('quality-drag-at-top', {
        body: await page.screenshot(),
        contentType: 'image/png',
      });
      await page.mouse.up();
      await expect(rows.first()).toHaveText('Quality 1');
      await expect(
        list.getByRole('switch', { name: 'Quality 1', exact: true }),
      ).toBeChecked();
      await expect(grip).toBeFocused();
    }
  }
  expect(fixture.errors).toEqual([]);
});
