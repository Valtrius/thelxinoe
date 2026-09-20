import { test, expect } from '@playwright/test';
test('the web bundle is served and setup is available', async ({ page }) => {
  await page.goto('/');
  await expect(
    page.getByRole('heading', { name: 'Welcome to Thelxinoe' }),
  ).toBeVisible();
  await expect(page.getByLabel('Setup code')).toHaveCount(0);
  await expect(page.getByLabel('Username', { exact: true })).toBeVisible();
  await expect(page.getByLabel('Password', { exact: true })).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Create your server' }),
  ).toBeVisible();
});
