import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test('available service releases show versions, container rebuilds and honest fallbacks', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  const image = `lscr.io/linuxserver/radarr@sha256:${'a'.repeat(64)}`;
  let release: {
    image: string;
    version: string | null;
    build_version: string | null;
    release_notes_url: string | null;
  } | null = {
    image,
    version: '6.4.4.10685',
    build_version: '6.4.4.10685-ls318',
    release_notes_url:
      'https://github.com/Radarr/Radarr/releases/tag/v6.4.4.10685',
  };
  let candidate = image;
  await page.route('**/api/v1/admin/service-updates', (route) =>
    route.fulfill({
      json: {
        policies: [
          {
            service_id: 'managed-radarr',
            policy: 'inherit',
            window_start: 0,
            window_end: 0,
            candidate,
            release,
            checked_at: 1790409600,
            error: null,
          },
        ],
        items: [],
        services: [{ id: 'managed-radarr', kind: 'radarr' }],
        timezone: 'UTC',
        server_policy: { policy: 'notify', window_start: 3, window_end: 5 },
      },
    }),
  );
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('link', { name: 'Radarr', exact: true })
    .click();
  const updates = page.getByRole('region', { name: 'Updates', exact: true });
  await expect(
    updates.getByText('Available version: 6.4.4.10685', { exact: true }),
  ).toBeVisible();
  await expect(
    updates.getByRole('link', { name: 'Release notes', exact: true }),
  ).toHaveAttribute('href', release.release_notes_url!);
  await expect(updates.getByText(image, { exact: true })).toBeHidden();
  await updates.getByText('Image details', { exact: true }).click();
  await expect(updates.getByText(image, { exact: true })).toBeVisible();
  await expect(
    updates.getByText('6.4.4.10685-ls318', { exact: true }),
  ).toBeVisible();
  await testInfo.attach('available-version', {
    body: await updates.screenshot(),
    contentType: 'image/png',
  });
  release = {
    ...release,
    version: '6.0.0',
    build_version: '6.0.0-ls300',
    release_notes_url: null,
  };
  await expect(
    updates.getByText('Container update available', { exact: true }),
  ).toBeVisible();
  await expect(
    updates.getByRole('link', { name: 'Release notes' }),
  ).toHaveCount(0);
  await testInfo.attach('container-update', {
    body: await updates.screenshot(),
    contentType: 'image/png',
  });
  release = null;
  await expect(
    updates.getByText('Available image:', { exact: false }),
  ).toBeVisible();
  await expect(updates.getByText(image, { exact: true })).toBeVisible();
  // Metadata belonging to a previous image must not survive a new discovery.
  release = {
    image: 'old-image',
    version: '99.0.0',
    build_version: null,
    release_notes_url: 'https://github.com/Radarr/Radarr/releases/tag/v99.0.0',
  };
  await page.reload();
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('link', { name: 'Radarr', exact: true })
    .click();
  await expect(
    updates.getByText('Available image:', { exact: false }),
  ).toBeVisible();
  await expect(
    updates.getByRole('link', { name: 'Release notes' }),
  ).toHaveCount(0);
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(updates.getByText(image, { exact: true })).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await testInfo.attach('missing-metadata-mobile', {
    body: await updates.screenshot(),
    contentType: 'image/png',
  });
  candidate = 'ghcr.io/example/radarr:stable';
  await expect(updates.getByText('Up to date.', { exact: true })).toBeVisible();
  await expect(
    updates.getByText('Available image:', { exact: false }),
  ).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
