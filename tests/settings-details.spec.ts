import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test('closing the avatar picker clears its edit overlay without a click elsewhere', async ({
  page,
}) => {
  const fixture = await installUiFixture(page);
  await page.goto('/');
  const input = page.getByLabel('Change profile picture');
  const avatar = input.locator('..');
  const avatarBounds = (await avatar.boundingBox())!;
  expect(avatarBounds.width).toBeCloseTo(avatarBounds.height, 0);
  const overlay = avatar.locator('span[aria-hidden="true"]');
  await avatar.hover();
  await expect(overlay).toHaveCSS('opacity', '1');
  const chooser = page.waitForEvent('filechooser');
  await avatar.click();
  await chooser;
  // Playwright intercepts native dialogs. Deliver the browser's cancel event
  // while the pointer is still over the avatar and the input retains focus.
  await input.focus();
  await input.dispatchEvent('cancel');
  await expect(overlay).toHaveCSS('opacity', '0');
  await expect(overlay).toHaveCSS('transition-duration', '0s');
  await expect(input).not.toBeFocused();
  await page.mouse.move(10, 10);
  await avatar.hover();
  await expect(overlay).toHaveCSS('opacity', '1');
  await page.mouse.move(10, 10);
  await expect(overlay).toHaveCSS('opacity', '0');
  expect(fixture.errors).toEqual([]);
});

test('adaptive settings keep navigation and service management accessible from mobile to 4K', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { role: 'admin' });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  const account = nav.getByRole('button', { name: 'Account', exact: true });
  await expect(account).toBeVisible();
  await expect(nav.getByText('Personal', { exact: true })).toBeVisible();
  await expect(nav.getByText('Administration', { exact: true })).toBeVisible();
  await expect(
    page.getByRole('heading', { name: 'Appearance', exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText('Use the server default or choose your own timezone', {
      exact: false,
    }),
  ).toHaveCount(0);
  await nav
    .getByRole('button', { name: 'Media services', exact: true })
    .click();
  const strip = page.getByRole('navigation', { name: 'Select service' });
  await expect(strip).toBeVisible();
  await expect(strip.getByRole('button').first()).toHaveAccessibleName('Seerr');
  await expect(strip.getByRole('button').nth(1)).toHaveAccessibleName(
    'Recyclarr',
  );
  await strip.getByRole('button', { name: 'Radarr', exact: true }).click();
  for (const size of [
    { width: 3840, height: 2160 },
    { width: 1440, height: 1000 },
    { width: 960, height: 800 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(size);
    for (const button of await nav.getByRole('button').all()) {
      const box = (await button.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(size.width);
    }
    expect(
      await strip.evaluate((node) => node.scrollWidth <= node.clientWidth),
    ).toBe(true);
    const rail = page.getByRole('complementary', { name: 'Service controls' });
    await expect(
      rail.getByRole('button', { name: 'Restart', exact: true }),
    ).toBeVisible();
    const title = (await rail
      .getByRole('heading', { name: 'Radarr', exact: true })
      .boundingBox())!;
    const restart = (await rail
      .getByRole('button', { name: 'Restart', exact: true })
      .boundingBox())!;
    expect(restart.y - title.y).toBeLessThan(220);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath(`adaptive-services-${size.width}.png`),
      fullPage: true,
    });
  }
  await expect(page.locator('.services-footer')).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('service labels stay inside their tabs across the sidebar breakpoint', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
    stackProvisionState: 'blocked',
  });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/');
  const strip = page.getByRole('navigation', { name: 'Select service' });
  await expect(strip.getByText('Setup blocked', { exact: true })).toBeVisible();
  for (const width of [
    720, 721, 760, 800, 900, 960, 1440, 1572, 3840, 390, 320,
  ]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const collapsed of width > 720 ? [false, true] : [true]) {
      if (width > 720) {
        const toggle = page.getByRole('button', {
          name: collapsed ? 'Collapse sidebar' : 'Expand sidebar',
          exact: true,
        });
        if (await toggle.isVisible()) await toggle.click();
        await expect(
          page.getByRole('button', {
            name: collapsed ? 'Expand sidebar' : 'Collapse sidebar',
            exact: true,
          }),
        ).toBeVisible();
      }
      await strip.scrollIntoViewIfNeeded();
      await expect(strip.getByRole('button')).toHaveCount(8);
      await page.screenshot({
        path: testInfo.outputPath(
          `service-tabs-${width}-${collapsed ? 'collapsed' : 'expanded'}.png`,
        ),
      });
      await expect
        .poll(
          async () =>
            strip.getByRole('button').evaluateAll((buttons) =>
              buttons.flatMap((button) => {
                const bounds = button.getBoundingClientRect();
                const issues: string[] = [];
                if (button.scrollWidth > button.clientWidth + 1)
                  issues.push(`${button.ariaLabel}: tab overflow`);
                for (const content of button.querySelectorAll(
                  'img, strong, .service-status',
                )) {
                  const box = content.getBoundingClientRect();
                  if (
                    box.left < bounds.left ||
                    box.right > bounds.right ||
                    box.top < bounds.top ||
                    box.bottom > bounds.bottom
                  ) {
                    issues.push(`${button.ariaLabel}: content outside tab`);
                  }
                }
                return issues;
              }),
            ),
          {
            message: `Service tabs at ${width}px with sidebar ${collapsed ? 'collapsed' : 'expanded'}`,
          },
        )
        .toEqual([]);
      expect(
        await strip.evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true);
      expect(
        await page
          .locator('.workspace-scroll')
          .evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true);
    }
  }
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('activity and audit filters preserve real actions and older-page queries', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'jobs',
  });
  await page.route('**/api/v1/admin/jobs', (route) =>
    route.request().method() === 'POST'
      ? route.fallback()
      : route.fulfill({
          json: {
            items: [
              { id: 'running', kind: 'Library scan', state: 'running' },
              { id: 'failed', kind: 'Guide sync', state: 'failed' },
              {
                id: 'complete',
                kind: 'Successful checkpoint',
                state: 'complete',
              },
              { id: 'queued', kind: 'Pending checkpoint', state: 'queued' },
            ],
          },
        }),
  );
  const queries: URL[] = [];
  await page.route('**/api/v1/admin/audit?*', (route) => {
    const url = new URL(route.request().url());
    queries.push(url);
    return route.fulfill({
      json: {
        actors: [{ id: 'layout-fixture', username: 'Layout viewer' }],
        actions: ['settings.update', 'user.create'],
        items:
          url.searchParams.get('action') === 'user.create'
            ? []
            : [
                {
                  id: url.searchParams.has('before') ? 1 : 2,
                  action: 'settings.update',
                  actor: 'Layout viewer',
                  target: url.searchParams.has('before')
                    ? 'Older server change'
                    : 'Server',
                  created_at: 1789984800,
                },
              ],
        next_before: url.searchParams.has('before') ? null : 2,
      },
    });
  });
  await page.goto('/');
  const jobState = page.getByLabel('Job state', { exact: true });
  await jobState.selectOption({ label: 'Completed' });
  await expect(
    page.getByText('Successful checkpoint', { exact: true }),
  ).toBeVisible();
  await expect(
    jobState.getByRole('option', { name: /^complete/i }),
  ).toHaveCount(1);
  await expect(
    page.getByText('1 of 4 latest jobs', { exact: true }),
  ).toBeVisible();
  await expect(page.getByText('Guide sync', { exact: true })).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath('completed-jobs.png'),
    fullPage: true,
  });
  for (const [label, kind] of [
    ['Queued', 'Pending checkpoint'],
    ['Running', 'Library scan'],
  ]) {
    await jobState.selectOption({ label });
    await expect(page.getByText(kind, { exact: true })).toBeVisible();
    await expect(
      page.getByText('Successful checkpoint', { exact: true }),
    ).toHaveCount(0);
  }
  await page.getByLabel('Job state', { exact: true }).selectOption('failed');
  await expect(page.getByText('Guide sync', { exact: true })).toBeVisible();
  await expect(page.getByText('Library scan', { exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Run checkpoint' }).click();
  await expect
    .poll(() => fixture.writes.some((write) => write.path === '/admin/jobs'))
    .toBe(true);
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Audit', exact: true })
    .click();
  await page
    .getByLabel('Audit user', { exact: true })
    .selectOption('layout-fixture');
  await page
    .getByLabel('Audit action', { exact: true })
    .selectOption('settings.update');
  await page.getByRole('button', { name: 'Load older activity' }).click();
  await expect(
    page.getByText('Older server change', { exact: true }),
  ).toBeVisible();
  expect(queries.at(-1)?.searchParams.get('user')).toBe('layout-fixture');
  expect(queries.at(-1)?.searchParams.get('action')).toBe('settings.update');
  expect(queries.at(-1)?.searchParams.get('before')).toBe('2');
  await page
    .getByLabel('Audit action', { exact: true })
    .selectOption('user.create');
  await expect(
    page.getByText('No activity matches these filters.'),
  ).toBeVisible();
  expect(queries.at(-1)?.searchParams.has('before')).toBe(false);
  await page.screenshot({
    path: testInfo.outputPath('filtered-audit.png'),
    fullPage: true,
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('settings sections adapt without hiding controls or stretching fields', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { role: 'admin' });
  const responses: Record<string, unknown> = {
    '/admin/segments': { config: { local: true, external: false }, items: [] },
    '/admin/backups': { items: [], destination: '/backups' },
    '/admin/retention': {
      policies: ['movies', 'shows'].map((domain) => ({
        domain,
        enabled: false,
        grace_seconds: 172800,
        exclude_specials: true,
        trigger_users: [],
      })),
      users: [{ id: 'layout-fixture', username: 'Layout viewer' }],
      roots: [],
      items: [],
    },
    '/admin/audit': { items: [], actors: [], actions: [], next_before: null },
  };
  await page.route('**/api/v1/**', (route) => {
    const path = new URL(route.request().url()).pathname.slice(
      '/api/v1'.length,
    );
    return path in responses
      ? route.fulfill({ json: responses[path] })
      : route.fallback();
  });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  await page
    .getByRole('searchbox', { name: 'Find a setting' })
    .fill('grace period');
  await nav.getByRole('button', { name: 'Retention', exact: true }).click();
  await expect(
    page.getByRole('searchbox', { name: 'Find a setting' }),
  ).toHaveValue('');
  for (const viewport of [
    { width: 1440, height: 1000, theme: 'Light theme' },
    { width: 390, height: 844, theme: 'Dark theme' },
    { width: 3840, height: 2160, theme: 'Dark theme' },
  ]) {
    await page.setViewportSize({
      width: viewport.width,
      height: viewport.height,
    });
    await page
      .getByRole('button', { name: viewport.theme, exact: true })
      .click();
    for (const name of [
      'Account',
      'Playback',
      'Server',
      'Episode analysis',
      'Retention',
      'Backups',
      'People',
      'Activity',
      'Audit',
    ]) {
      await nav.getByRole('button', { name, exact: true }).click();
      await expect(page.locator('.settings-content > h1')).toHaveText(name);
      await expect(
        page
          .locator(
            '.settings-content .panel, .settings-content .settings-section',
          )
          .first(),
      ).toBeVisible();
      if (name === 'Retention')
        await expect(
          page.getByRole('button', { name: 'Save movie policy' }),
        ).toBeVisible();
      await page.locator('.workspace-scroll').evaluate((node) => {
        node.scrollTop = 0;
      });
      expect(
        await page
          .locator('.workspace-scroll')
          .evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true);
      if (name === 'Account' && viewport.width === 3840) {
        const panels = await page
          .locator('.settings-panels > .panel')
          .evaluateAll((nodes) =>
            nodes.map((node) => node.getBoundingClientRect().x),
          );
        expect(new Set(panels).size).toBe(3);
      }
      expect(
        await page
          .locator('.settings-content select')
          .evaluateAll((nodes) =>
            nodes.every((node) => node.getBoundingClientRect().width <= 289),
          ),
      ).toBe(true);
      await page.screenshot({
        path: testInfo.outputPath(
          `${name.toLowerCase().replaceAll(' ', '-')}-${viewport.width}.png`,
        ),
        fullPage: true,
      });
      if (viewport.width === 390) {
        await page.locator('.settings-content > h1').scrollIntoViewIfNeeded();
        await page.screenshot({
          path: testInfo.outputPath(
            `${name.toLowerCase().replaceAll(' ', '-')}-390-content.png`,
          ),
          fullPage: true,
        });
      }
    }
  }
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

for (const admin of [false, true]) {
  test(`unconfigured online accounts show ${admin ? 'an administrator setup link' : 'guidance for members'}`, async ({
    page,
  }) => {
    const fixture = await installUiFixture(page, {
      role: admin ? 'admin' : 'user',
      settingsSection: 'online',
    });
    await page.route('**/api/v1/**', async (route) => {
      const path = new URL(route.request().url()).pathname.replace(
        '/api/v1',
        '',
      );
      if (path === '/online/youtube' || path === '/online/twitch')
        return route.fulfill({
          json: {
            configured: false,
            linking_available: true,
            account: { status: 'disconnected' },
          },
        });
      if (path === '/online/kick')
        return route.fulfill({
          json: { configured: false, connected: false, items: [] },
        });
      if (path.startsWith('/admin/online'))
        return route.fulfill({
          json: {
            configured: false,
            google_configured: false,
            youtube_downloads: false,
            youtube_daily_quota: 10000,
            quota: { used: 0, blocked: false },
          },
        });
      return route.fallback();
    });
    await page.goto('/');
    const text =
      'Connections unavailable until an admin sets up the provider applications.';
    const notice = page.getByText(text, { exact: true });
    await expect(notice).toBeVisible();
    const rail = notice.locator('xpath=ancestor-or-self::*[@data-tone][1]');
    await expect(rail).toHaveCSS('border-left-width', '3px');
    await expect(
      page.getByRole('button', { name: 'Connect YouTube', exact: true }),
    ).toBeDisabled();
    if (admin) {
      await page.getByRole('link', { name: text }).click();
      await expect(
        page.getByRole('heading', { name: 'YouTube application' }),
      ).toBeVisible();
      await expect(
        page
          .getByRole('navigation', { name: 'Settings navigation' })
          .getByRole('button', { name: 'Provider applications' }),
      ).toHaveAttribute('aria-current', 'page');
    } else {
      await expect(page.getByRole('link', { name: text })).toHaveCount(0);
      await page.screenshot({
        path: 'test-results/online-accounts-guidance.png',
        fullPage: true,
      });
    }
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('administrator setup is centered and accepts an eight-character password', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { signedIn: false });
  let submitted: unknown;
  await page.route('**/api/v1/setup', async (route) => {
    if (route.request().method() === 'POST') {
      submitted = route.request().postDataJSON();
      return route.fulfill({
        json: {
          user: {
            id: 'layout-fixture',
            username: 'admin',
            role: 'admin',
            timezone: 'UTC',
          },
        },
      });
    }
    return route.fulfill({ json: { setup_required: true } });
  });
  await page.goto('/');
  await expect(
    page.getByRole('heading', { name: 'Welcome to Thelxinoe' }),
  ).toBeVisible();
  for (const viewport of [
    { width: 1440, height: 1000 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport);
    const card = (await page.locator('.auth-card').boundingBox())!;
    expect(
      Math.abs(card.x + card.width / 2 - viewport.width / 2),
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(card.y + card.height / 2 - viewport.height / 2),
    ).toBeLessThanOrEqual(1);
    await page.screenshot({
      path: `test-results/setup-centered-${viewport.width}.png`,
      fullPage: true,
    });
  }
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page.getByLabel('Password', { exact: true }).fill('12345678');
  await page.getByLabel('Confirm password', { exact: true }).fill('12345678');
  await page.getByRole('button', { name: 'Create your server' }).click();
  await expect
    .poll(() => submitted)
    .toEqual({ username: 'admin', password: '12345678' });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
