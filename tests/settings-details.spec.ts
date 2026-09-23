import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test('closing the avatar picker clears its edit overlay without a click elsewhere', async ({
  page,
}) => {
  const fixture = await installUiFixture(page);
  await page.goto('/');
  const input = page.getByLabel('Change profile picture');
  const avatar = input.locator('..');
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

test('account navigation starts flush and media service tabs meet the workspace edge', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { role: 'admin' });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  const account = nav.getByRole('button', { name: 'Account', exact: true });
  await expect(account).toBeVisible();
  expect((await account.boundingBox())!.y).toBe((await nav.boundingBox())!.y);
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
  for (const size of [
    { width: 1440, height: 1000 },
    { width: 960, height: 800 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(size);
    await expect
      .poll(() =>
        strip.evaluate((element) => {
          const content = element
            .closest('.settings-content')!
            .getBoundingClientRect();
          const tabs = element.getBoundingClientRect();
          return Math.max(
            Math.abs(tabs.x - content.x),
            Math.abs(tabs.y - content.y),
          );
        }),
      )
      .toBeLessThanOrEqual(1);
    const rail = page.getByRole('complementary', { name: 'Service controls' });
    expect(
      await rail.evaluate((node) =>
        parseFloat(getComputedStyle(node).paddingLeft),
      ),
    ).toBeGreaterThanOrEqual(12);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `test-results/services-insets-${size.width}.png`,
      fullPage: true,
    });
  }
  await expect(page.locator('.services-footer')).toHaveCount(0);
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

test('notification counts sit on the bell in both sidebar sizes and clear when read', async ({
  page,
}) => {
  const fixture = await installUiFixture(page);
  let read = false;
  await page.route('**/api/v1/me/notifications**', async (route) => {
    if (route.request().method() === 'PUT') read = true;
    await route.fulfill({
      json: {
        items: [1, 2, 3].map((id) => ({
          id: String(id),
          severity: 'info',
          message: `Notice ${id}`,
          created_at: 1700000000,
          read_at: read ? 1700000100 : null,
        })),
      },
    });
  });
  await page.goto('/');
  const button = page.getByRole('button', {
    name: 'Notifications',
    exact: true,
  });
  const badge = button.locator('[data-notification-count]');
  await expect(badge).toHaveText('3');
  await expect(button).not.toContainText('(');
  for (const toggle of ['Collapse sidebar', 'Expand sidebar']) {
    await page.getByRole('button', { name: toggle }).click();
    const icon = (await button.locator('svg').boundingBox())!;
    const count = (await badge.boundingBox())!;
    expect(
      Math.abs(count.x + count.width / 2 - icon.x - icon.width),
    ).toBeLessThan(10);
    expect(Math.abs(count.y + count.height / 2 - icon.y)).toBeLessThan(10);
  }
  await button.click();
  await page
    .getByRole('button', { name: 'Mark all read', exact: true })
    .click();
  await expect(badge).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

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
