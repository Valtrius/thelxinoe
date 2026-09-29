import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test('sign-in rechecks setup and requires confirmation before creating an account', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { signedIn: false });
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('local test passphrase');
  let created = false;
  await page.route('**/api/v1/setup', async (route) => {
    if (route.request().method() === 'POST') {
      created = true;
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
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(
    page.getByLabel('Confirm password', { exact: true }),
  ).toBeVisible();
  expect(created).toBe(false);
  expect(fixture.writes.some((write) => write.path === '/auth/login')).toBe(
    false,
  );
  await page
    .getByLabel('Confirm password', { exact: true })
    .fill('local test passphrase');
  await page
    .getByRole('button', { name: 'Create your server', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Sign out', exact: true }),
  ).toBeVisible();
  expect(created).toBe(true);
  expect(fixture.errors).toEqual([]);
  await testInfo.attach('account-created', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('sign-in checks compatibility before sending credentials', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { signedIn: false });
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('local test passphrase');
  await page.route('**/api/v1/health', (route) =>
    route.fulfill({ json: { api_version: 99, api_min: 99, api_max: 99 } }),
  );
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'Update required', exact: true }),
  ).toBeVisible();
  expect(fixture.writes.some((write) => write.path === '/auth/login')).toBe(
    false,
  );
  expect(fixture.errors).toEqual([]);
  await testInfo.attach('incompatible', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});
