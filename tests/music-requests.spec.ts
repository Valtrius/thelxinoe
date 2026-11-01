import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

const album = (index: number, title: string, artist: string) => ({
  external_id: `00000000-0000-4000-8000-${String(index).padStart(12, '0')}`,
  title,
  artist,
  year: 2026,
  cover: `/api/v1/acquisition/music/covers/${index}`,
  in_library: false,
  request: null,
});

test('music requests work without Seerr, from the Music tab to Requests', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'user',
    section: 'Discover',
  });
  const requested: unknown[] = [];
  const requests: unknown[] = [];
  await page.route('**/api/v1/seerr/status', (route) =>
    route.fulfill({ json: { configured: false, ready: false } }),
  );
  await page.route('**/api/v1/acquisition/services', (route) =>
    route.fulfill({
      json: {
        items: [
          { id: 'manager-lidarr', name: 'Lidarr', kind: 'lidarr', ready: true },
        ],
      },
    }),
  );
  await page.route('**/api/v1/acquisition/music/covers/*', (route) =>
    route.fulfill({
      contentType: 'image/svg+xml',
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#466854"/></svg>',
    }),
  );
  await page.route(
    (url) =>
      /^\/api\/v1\/acquisition\/music\/(artists|trending|fresh)$/.test(
        url.pathname,
      ),
    (route) => {
      const feed = new URL(route.request().url()).pathname.split('/').pop();
      return route.fulfill({
        json: {
          items:
            feed === 'artists'
              ? [album(1, 'Long Exposure', 'Harbor Lights')]
              : feed === 'trending'
                ? [
                    {
                      ...album(2, 'Slow Weather', 'Low Tide'),
                      in_library: true,
                    },
                    album(3, 'Night Ferry', 'Copper Sky'),
                  ]
                : [],
        },
      });
    },
  );
  await page.route('**/api/v1/acquisition/requests', (route) => {
    if (route.request().method() === 'POST') {
      requested.push(route.request().postDataJSON());
      requests.unshift({
        id: 'request-1',
        title: 'Night Ferry',
        state: 'pending',
        created_at: 1,
        error: null,
        username: 'Preview user',
        user_id: 'layout-fixture',
        attention: null,
      });
      return route.fulfill({ json: { id: 'request-1', state: 'pending' } });
    }
    return route.fulfill({ json: { items: requests, page: 1, pages: 1 } });
  });
  await page.goto('/');
  const tabs = page.getByRole('navigation', { name: 'Discover navigation' });
  // Without Seerr, Discover opens where requests still work.
  await expect(tabs.getByRole('button', { name: 'Music' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  const trending = page.getByRole('region', { name: 'Trending albums' });
  await expect(
    trending.getByRole('article', { name: 'Slow Weather' }),
  ).toContainText('In library');
  await expect(
    page.getByRole('region', { name: 'Fresh releases' }),
  ).toHaveCount(0);
  await trending.getByRole('button', { name: 'Request Night Ferry' }).click();
  await expect(
    trending.getByRole('article', { name: 'Night Ferry' }),
  ).toContainText('Pending approval');
  expect(requested).toEqual([
    {
      service_id: 'manager-lidarr',
      external_id: '00000000-0000-4000-8000-000000000003',
    },
  ]);
  await page.screenshot({ path: testInfo.outputPath('music-tab.png') });

  await tabs.getByRole('button', { name: 'Movies' }).click();
  await expect(
    page.getByRole('heading', { name: /Connect Seerr/ }),
  ).toBeVisible();

  await tabs.getByRole('button', { name: /My requests/ }).click();
  const music = page.getByRole('region', { name: 'Music requests' });
  await expect(music).toContainText('Night Ferry');
  await expect(music).toContainText('Pending approval');
  await expect(
    page.getByRole('heading', { name: /Connect Seerr/ }),
  ).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath('music-requests.png') });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
