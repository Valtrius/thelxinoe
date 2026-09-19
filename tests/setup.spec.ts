import { test, expect } from '@playwright/test';
test('the web bundle is served and setup is available', async ({ page }) => {
  await page.goto('/');
  await expect(
    page.getByRole('heading', { name: 'Welcome to Thelxinoe' }),
  ).toBeVisible();
  await expect(page.getByLabel('Setup code')).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Create your server' }),
  ).toBeVisible();
});
