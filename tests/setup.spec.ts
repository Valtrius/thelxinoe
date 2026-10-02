import { test, expect } from '@playwright/test';
test.describe.configure({ mode: 'serial' });
test('the web bundle is served and setup is available', async ({ page }) => {
  await page.goto('/');
  await expect(
    page.getByRole('heading', { name: 'Welcome to Thelxinoe' }),
  ).toBeVisible();
  await expect(page.getByLabel('Setup code')).toHaveCount(0);
  await expect(page.getByLabel('Username', { exact: true })).toBeVisible();
  await expect(page.getByLabel('Password', { exact: true })).toBeVisible();
  await expect(
    page.getByLabel('Confirm password', { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText('YOUR MEDIA. YOUR PLACE.', { exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText('A shared library, a space of your own.', { exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Create your server' }),
  ).toBeVisible();
});

test('administrator passwords must match before setup is submitted', async ({
  page,
  request,
}) => {
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('a long test passphrase');
  await page
    .getByLabel('Confirm password', { exact: true })
    .fill('a different test passphrase');
  await page
    .getByRole('button', { name: 'Create your server', exact: true })
    .click();
  await expect(page.getByRole('alert')).toHaveText('Passwords do not match.');
  expect(
    (await (await request.get('/api/v1/setup')).json()).setup_required,
  ).toBe(true);
});

test('matching passwords create the administrator and empty provider pages load', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('test-only long passphrase');
  await page
    .getByLabel('Confirm password', { exact: true })
    .fill('test-only long passphrase');
  await page
    .getByRole('button', { name: 'Create your server', exact: true })
    .click();
  await expect(
    page.getByRole('navigation', { name: 'Main navigation' }),
  ).toBeVisible();
  for (const [name, heading] of [
    ['YouTube', 'Connect YouTube'],
    ['Twitch', 'Connect Twitch'],
    ['Kick', 'Track Kick channels'],
  ]) {
    await page.getByRole('button', { name, exact: true }).click();
    await expect(
      page.getByRole('heading', { name: heading, exact: true }),
    ).toBeVisible();
  }
  expect(errors).toEqual([]);
});

test('administrators filter the persisted audit trail by actor and action', async ({
  page,
}, testInfo) => {
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('test-only long passphrase');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Main navigation' })
    .getByRole('button', { name: 'Settings', exact: true })
    .click();
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  await nav.getByRole('button', { name: 'People', exact: true }).click();
  const form = page.getByRole('form', { name: 'Create user' });
  await form
    .getByLabel('Username', { exact: true })
    .fill('audit-filter-member');
  await form
    .getByLabel('Password', { exact: true })
    .fill('audit-only-passphrase');
  await form.getByRole('button', { name: 'Add user', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'User access' }),
  ).toBeVisible();
  await expect(
    page.getByText('audit-filter-member', { exact: true }).first(),
  ).toBeVisible();
  await nav.getByRole('button', { name: 'Audit', exact: true }).click();
  await page
    .getByLabel('Audit user', { exact: true })
    .selectOption({ label: 'admin' });
  await page
    .getByLabel('Audit action', { exact: true })
    .selectOption('user.create');
  await expect(page.locator('.settings-content .row')).toHaveCount(1);
  await expect(page.locator('.settings-content .row')).toContainText(
    'user.create',
  );
  await page.getByLabel('Audit action', { exact: true }).selectOption('setup');
  await expect(page.locator('.settings-content .row')).toHaveCount(1);
  await expect(page.locator('.settings-content .row')).toContainText('setup');
  await page.screenshot({
    path: testInfo.outputPath('persisted-audit-filter.png'),
    fullPage: true,
  });
});
