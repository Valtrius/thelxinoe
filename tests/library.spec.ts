import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { budgets, waitForServerTools } from '../scripts/ci-readiness.mjs';
test('movie, multi-episode and tagged music scan into browsable libraries with stable identities', async ({
  page,
}) => {
  const watchTimeout = 15_000;
  test.setTimeout(budgets.provision + 60_000);
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('test-only long passphrase');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Refresh Home', exact: true }),
  ).toBeVisible();
  const api = page.request;
  await waitForServerTools(async (timeout) => {
    const response = await api.get('/api/v1/admin/tools', { timeout });
    expect(response.ok()).toBe(true);
    return response.json();
  });
  const headers = { 'X-Thelxinoe-Client': '1' };
  const existing = (await (await api.get('/api/v1/catalog/roots')).json())
    .items as { path: string }[];
  for (const kind of ['movies', 'shows', 'music']) {
    if (!existing.some((root) => root.path === `/media/${kind}`)) {
      const response = await api.post('/api/v1/catalog/roots', {
        headers,
        data: { name: `Fixture ${kind}`, kind, path: `/media/${kind}` },
      });
      expect(response.status()).toBe(200);
    }
  }
  await expect
    .poll(async () =>
      Promise.all(
        ['track', 'episode'].map(
          async (kind) =>
            (
              (await (await api.get(`/api/v1/catalog?kind=${kind}`)).json())
                .items as unknown[]
            ).length,
        ),
      ),
    )
    .toEqual([1, 3]);
  const movies = (
    await (await api.get('/api/v1/catalog?kind=movie')).json()
  ).items.filter((item: { available: boolean }) => item.available);
  expect(movies).toHaveLength(1);
  expect(movies[0]).toMatchObject({
    title: 'Thelxinoe Fixture',
    year: 2020,
    available: true,
  });
  const movieId = movies[0].id;
  const detail = await (await api.get(`/api/v1/catalog/${movieId}`)).json();
  expect(detail.files).toHaveLength(1);
  expect(detail.local_trailers).toHaveLength(1);
  expect(
    (await (await api.get('/api/v1/catalog?kind=episode')).json()).items,
  ).toHaveLength(3);
  const track = (await (await api.get('/api/v1/catalog?kind=track')).json())
    .items[0];
  expect(track.title).toBe('Fixture Track');
  const root = (
    await (await api.get('/api/v1/catalog/roots')).json()
  ).items.find((root: { kind: string }) => root.kind === 'movies');
  const scan = await api.post(`/api/v1/catalog/roots/${root.id}/scan`, {
    headers,
  });
  const jobId = (await scan.json()).job_id;
  await expect
    .poll(async () => {
      const jobs = (await (await api.get('/api/v1/admin/jobs')).json()).items;
      return jobs.find((job: { id: string }) => job.id === jobId)?.state;
    })
    .toBe('complete');
  expect(
    (await (await api.get('/api/v1/catalog?kind=movie')).json()).items[0].id,
  ).toBe(movieId);
  await page
    .getByRole('navigation', { name: 'Main navigation' })
    .getByRole('link', { name: 'Movies', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Thelxinoe Fixture 2020' }),
  ).toBeVisible();
  const cleared = await api.put(`/api/v1/catalog/${movieId}/state`, {
    headers,
    data: { watch_later: false },
  });
  expect(cleared.status()).toBe(200);
  await page.getByRole('button', { name: 'Thelxinoe Fixture 2020' }).click();
  await expect(
    page.getByRole('heading', {
      name: 'Thelxinoe Fixture',
      level: 1,
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.locator('[data-page-content]')).toHaveAttribute(
    'data-page-phase',
    'idle',
  );
  await expect(
    page.getByRole('button', { name: 'Play', exact: true }),
  ).toBeVisible();
  const later = page.getByRole('button', { name: 'Watch Later', exact: true });
  await later.click();
  await expect(later).toHaveAttribute('aria-pressed', 'true');
  await page.reload();
  await expect(
    page.getByRole('button', { name: 'Watch Later', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true');
  await page
    .getByRole('button', { name: 'Back to movies', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Thelxinoe Fixture 2020' }),
  ).toBeVisible();
  const show = (await (await api.get('/api/v1/catalog?kind=show')).json())
    .items[0];
  const seasons = (
    await (await api.get(`/api/v1/catalog/${show.id}/seasons`)).json()
  ).seasons as { number: number; episodes: { id: string | null }[] }[];
  expect(
    seasons.flatMap((season) => season.episodes).filter((e) => e.id),
  ).toHaveLength(3);
  await page.goto(`/#library/shows/${show.id}`);
  await expect(page.getByRole('tablist', { name: 'Seasons' })).toBeVisible();
  await expect(
    page.getByRole('button', { name: /^Play episode / }).first(),
  ).toBeVisible();
  await page.screenshot({ path: '.local/library.png', fullPage: true });
  // Change files on Linux so this checks the real watcher on Docker Desktop too.
  // Host bind-mount events can otherwise wait for five-minute reconciliation.
  const watchedCopy = '/media/movies/Watch Fixture (2020).mp4';
  const change = (action: string) =>
    execFileSync(
      'docker',
      [
        'compose',
        '-f',
        'compose.test.yaml',
        'exec',
        '-T',
        'server',
        'python3',
        '-c',
        action,
      ],
      { stdio: 'pipe' },
    );
  change(
    `from shutil import copyfile; copyfile('/media/movies/Thelxinoe Fixture (2020).mp4', ${JSON.stringify(watchedCopy)})`,
  );
  let copiedId: string | undefined;
  try {
    await expect
      .poll(
        async () => {
          const movies = (
            await (await api.get('/api/v1/catalog?kind=movie')).json()
          ).items;
          const copy = movies.find(
            (m: { title: string }) => m.title === 'Watch Fixture',
          );
          copiedId = copy?.id;
          return copy?.available;
        },
        { timeout: watchTimeout, intervals: [500, 1000, 3000, 5000] },
      )
      .toBe(true);
    expect(copiedId).not.toBe(movieId);
  } finally {
    change(
      `from pathlib import Path; Path(${JSON.stringify(watchedCopy)}).unlink()`,
    );
  }
  await expect
    .poll(
      async () =>
        (await (await api.get(`/api/v1/catalog/${copiedId}`)).json()).available,
      { timeout: watchTimeout, intervals: [500, 1000, 3000, 5000] },
    )
    .toBe(false);
});
