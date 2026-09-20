import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const origin = 'https://localhost:26443';
const browser = await chromium.launch();
const context = await browser.newContext({
  ignoreHTTPSErrors: true,
  viewport: { width: 1440, height: 1000 },
});
async function api(path, method = 'GET', data) {
  const response = await context.request.fetch(origin + '/api/v1' + path, {
    method,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
    timeout: 120000,
  });
  const value = await response.json();
  if (!response.ok())
    throw Error(`${path}: ${response.status()} ${value.error?.message ?? ''}`);
  return value;
}
try {
  const credentials = {
    username: 'admin',
    password: 'test-only long passphrase',
  };
  if ((await api('/setup')).setup_required)
    await api('/setup', 'POST', credentials);
  else await api('/auth/login', 'POST', credentials);
  const roots = (await api('/catalog/roots')).items;
  const root =
    roots.find((r) => r.path === '/data/shows') ??
    (await api('/catalog/roots', 'POST', {
      name: 'Segment fixtures',
      kind: 'shows',
      path: '/data/shows',
    }));
  const job = (await api(`/catalog/roots/${root.id}/scan`, 'POST')).job_id;
  await expect
    .poll(
      async () =>
        (await api('/admin/jobs')).items.find((j) => j.id === job)?.state,
      { timeout: 60000 },
    )
    .toBe('complete');
  const episodes = (await api('/catalog?kind=episode')).items
    .filter((e) => e.title.startsWith('Segment Fixture'))
    .sort((a, b) => a.title.localeCompare(b.title));
  expect(episodes).toHaveLength(2);
  await expect
    .poll(
      async () => {
        const rows = (await api('/admin/segments')).items.filter((r) =>
          episodes.some((e) => e.id === r.media_id),
        );
        const failure = rows.find((r) => r.state === 'failed');
        if (failure) throw Error(failure.error);
        return rows.filter((r) => r.state === 'complete').length;
      },
      { timeout: 120000, intervals: [2000] },
    )
    .toBe(2);
  const results = [];
  for (const [index, episode] of episodes.entries()) {
    const data = await api(`/catalog/${episode.id}/segments`);
    for (const [kind, start, end] of [
      ['Intro', index === 0 ? 12 : 22, index === 0 ? 42 : 52],
      ['Credits', index === 0 ? 205 : 198, index === 0 ? 235 : 228],
    ]) {
      const found = data.items.find((s) => s.kind === kind);
      expect(found, `${episode.title} ${kind}`).toBeTruthy();
      expect(Math.abs(found.start - start)).toBeLessThan(6);
      expect(Math.abs(found.end - end)).toBeLessThan(6);
      expect(found.source).toBe('local');
    }
    results.push({ id: episode.id, title: episode.title, ...data });
  }
  const auth =
    'MediaBrowser Client="Segments fixture", Device="Automated test", DeviceId="segments-fixture", Version="1"';
  const loginResponse = await context.request.post(
    origin + '/Users/AuthenticateByName',
    {
      headers: { Authorization: auth },
      data: { Username: credentials.username, Pw: credentials.password },
    },
  );
  expect(loginResponse.status()).toBe(200);
  const login = await loginResponse.json();
  const compat = await context.request.get(
    origin + `/MediaSegments/${episodes[0].id}?includeSegmentTypes=Intro`,
    { headers: { 'X-Emby-Token': login.AccessToken } },
  );
  expect(compat.status()).toBe(200);
  const compatBody = await compat.json();
  expect(compatBody.Items).toHaveLength(1);
  expect(compatBody.Items[0].Type).toBe('Intro');
  expect(compatBody.Items[0].StartTicks).toBe(
    Math.round(
      results[0].items.find((s) => s.kind === 'Intro').start * 10000000,
    ),
  );
  const page = await context.newPage();
  await page.goto(origin);
  await page
    .getByRole('button', { name: 'Shows', exact: true })
    .first()
    .click();
  await page
    .locator('button.media-card')
    .filter({ hasText: 'Segment Fixture' })
    .click();
  const season = await api(`/catalog/${episodes[0].parent_id}`);
  await page
    .locator('button.media-card')
    .filter({ hasText: season.title })
    .click();
  await page
    .locator('button.media-card')
    .filter({ hasText: episodes[0].title })
    .click();
  const intro = results[0].items.find((s) => s.kind === 'Intro');
  for (const mode of ['Ask', 'Auto', 'Ignore']) {
    await api(`/catalog/${episodes[0].id}/state`, 'PUT', { watched: false });
    await api('/me/segments', 'PUT', {
      Intro: mode,
      Recap: 'Ask',
      Credits: 'Ask',
      Preview: 'Ask',
    });
    await page.getByRole('button', { name: 'Play media', exact: true }).click();
    await expect
      .poll(() => page.locator('video').evaluate((v) => v.readyState))
      .toBeGreaterThanOrEqual(2);
    await page.locator('video').evaluate((v, t) => {
      v.currentTime = t;
      return v.play();
    }, intro.start + 1);
    if (mode === 'Ask') {
      await page
        .getByRole('button', { name: 'Skip intro', exact: true })
        .click();
      await expect
        .poll(() => page.locator('video').evaluate((v) => v.currentTime))
        .toBeGreaterThanOrEqual(intro.end);
    }
    if (mode === 'Auto')
      await expect
        .poll(() => page.locator('video').evaluate((v) => v.currentTime))
        .toBeGreaterThanOrEqual(intro.end);
    if (mode === 'Ignore') {
      await expect(
        page.getByRole('button', { name: 'Skip intro', exact: true }),
      ).toHaveCount(0);
      expect(
        await page.locator('video').evaluate((v) => v.currentTime),
      ).toBeLessThan(intro.end);
    }
    await page
      .getByRole('button', { name: 'Close player', exact: true })
      .click();
  }
  const editor = page.locator('details.segments');
  await editor.locator('summary').click();
  await editor.getByLabel('Start', { exact: true }).first().fill('2');
  await editor.getByLabel('End', { exact: true }).first().fill('8');
  await editor
    .getByRole('button', { name: 'Save corrections', exact: true })
    .click();
  await expect(editor.getByRole('status')).toContainText(
    'Manual corrections saved',
  );
  const manual = await api(`/catalog/${episodes[0].id}/segments`);
  expect(manual.items[0].source).toBe('manual');
  expect(manual.items[0].start).toBe(2);
  await editor.screenshot({
    path: '.local/segments-editor.png',
    animations: 'disabled',
  });
  await editor
    .getByRole('button', { name: 'Use automatic timestamps', exact: true })
    .click();
  await expect(editor.getByRole('status')).toContainText(
    'Automatic timestamps restored',
  );
  await api('/me/segments', 'PUT', {
    Intro: 'Ask',
    Recap: 'Ask',
    Credits: 'Ask',
    Preview: 'Ask',
  });
  writeFileSync(
    '.local/segments-result.json',
    JSON.stringify(
      {
        passed: true,
        local: results,
        jellyfin: true,
        browserModes: ['Ask', 'Auto', 'Ignore'],
        manualCorrection: true,
      },
      null,
      2,
    ),
  );
  console.log(
    'Recurring audio, Jellyfin segments, browser skip modes and manual corrections passed',
  );
} finally {
  await browser.close();
}
