// Validate the real isolated services without downloading third-party media.
import { chromium, expect } from '@playwright/test';
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
async function api(path, method = 'GET', data) {
  const response = await context.request.fetch(
    'https://localhost:23443/api/v1' + path,
    { method, data, headers: { 'X-Thelxinoe-Client': '1' } },
  );
  if (!response.ok()) throw Error(`${path}: HTTP ${response.status()}`);
  return response.json();
}
async function manager(kind, port, version, path) {
  const key = readFileSync(
    `.local/acquisition/${kind}/config.xml`,
    'utf8',
  ).match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
  const response = await fetch(
    `http://localhost:${port}/api/v${version}/${path}`,
    { headers: { 'X-Api-Key': key } },
  );
  if (!response.ok) throw Error(`${kind}: HTTP ${response.status}`);
  return response.json();
}
const results = [];
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const services = (await api('/admin/support')).items;
  const prowlarr = services.find((s) => s.kind === 'prowlarr'),
    bazarr = services.find((s) => s.kind === 'bazarr'),
    nzbget = services.find((s) => s.kind === 'nzbget');
  const indexer = (await api('/admin/support/' + prowlarr.id)).indexers.find(
    (i) => i.name === 'Generated fixtures',
  );
  for (const action of ['test', 'disable', 'enable']) {
    await api('/admin/support/' + prowlarr.id, 'POST', {
      action,
      item_id: indexer.id,
    });
    if (action !== 'test')
      expect(
        (await api('/admin/support/' + prowlarr.id)).indexers.find(
          (i) => i.id === indexer.id,
        ).enabled,
      ).toBe(action === 'enable');
    results.push({ kind: 'prowlarr', action, passed: true });
  }
  const requests = (await api('/acquisition/requests')).items;
  for (const title of ['The Matrix', 'Firefly', 'Discovery']) {
    const row = requests.find(
      (r) => r.title === title && r.state === 'requested',
    );
    if (!row) throw Error(`Missing request for ${title}`);
    const releasePath = '/admin/acquisition/requests/' + row.id + '/releases';
    const releases = (
      await api(releasePath + (row.kind === 'sonarr' ? '?season_number=1' : ''))
    ).items;
    const release = releases.find((r) => r.title.endsWith('FIXTURE2'));
    expect(release).toBeTruthy();
    if (release.approved && !release.rejections?.length)
      await api(releasePath, 'POST', {
        guid: release.guid,
        indexer_id: release.indexer_id,
        season_number: row.kind === 'sonarr' ? 1 : undefined,
      });
    if (row.kind === 'radarr')
      await expect
        .poll(
          async () =>
            (await manager('radarr', 27878, 3, 'movie/' + row.manager_id))
              .hasFile,
          { timeout: 120000 },
        )
        .toBe(true);
    if (row.kind === 'sonarr')
      await expect
        .poll(
          async () =>
            (
              await manager(
                'sonarr',
                28989,
                3,
                'episodefile?seriesId=' + row.manager_id,
              )
            ).length,
          { timeout: 120000 },
        )
        .toBeGreaterThan(0);
    if (row.kind === 'lidarr')
      await expect
        .poll(
          async () => {
            const a = await manager(
              'lidarr',
              28686,
              1,
              'album/' + row.manager_id,
            );
            return (
              a.statistics?.trackFileCount === a.statistics?.trackCount &&
              a.statistics?.trackCount > 0
            );
          },
          { timeout: 120000 },
        )
        .toBe(true);
    results.push({
      kind: row.kind,
      generated_release: release.title,
      imported: true,
    });
  }
  if (!process.argv.includes('--downloads-only')) {
    const wanted = await api('/admin/support/' + bazarr.id);
    if (wanted.movies.some((m) => m.movie_id === 1))
      await api('/admin/support/' + bazarr.id, 'POST', {
        action: 'subtitles',
        item_id: 1,
        domain: 'movies',
        language: 'en',
      });
    await expect
      .poll(
        async () =>
          (await api('/admin/support/' + bazarr.id)).movies.some(
            (m) => m.movie_id === 1,
          ),
        { timeout: 60000 },
      )
      .toBe(false);
    const subtitles = readdirSync('.local/acquisition/data/movies', {
      recursive: true,
    }).filter((p) => p.endsWith('.en.srt'));
    expect(subtitles.length).toBeGreaterThan(0);
    expect(
      subtitles.some((p) =>
        readFileSync('.local/acquisition/data/movies/' + p, 'utf8').includes(
          'Generated subtitle fixture.',
        ),
      ),
    ).toBe(true);
    results.push({ kind: 'bazarr', generated_subtitles_saved: true });
  }
  const history = (await api('/admin/support/' + nzbget.id)).history;
  expect(
    history.filter((r) => r.title?.endsWith('FIXTURE2')).length,
  ).toBeGreaterThanOrEqual(3);
  results.push({
    kind: 'nzbget',
    completed_generated_downloads: history.filter((r) =>
      r.title?.endsWith('FIXTURE2'),
    ).length,
  });
  writeFileSync(
    '.local/acquisition-downloads-result.json',
    JSON.stringify(results, null, 2),
  );
  console.log(JSON.stringify(results));
} finally {
  await browser.close();
}
