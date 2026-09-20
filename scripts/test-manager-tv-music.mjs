// Generated files imported into the isolated Sonarr/Lidarr fixture only.
import { request, expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
const api = await request.newContext({
  baseURL: 'https://localhost:23443',
  ignoreHTTPSErrors: true,
  extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
});
async function call(path, method = 'GET', data) {
  const r = await api.fetch(`/api/v1${path}`, {
    method,
    data,
    timeout: 120000,
  });
  const body = await r.json();
  if (!r.ok())
    throw Error(`${path}: HTTP ${r.status()} ${body.error?.message}`);
  return body;
}
async function manager(kind, port, version, path) {
  const key = readFileSync(
    `.local/acquisition/${kind}/config.xml`,
    'utf8',
  ).match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
  const r = await fetch(`http://127.0.0.1:${port}/api/v${version}/${path}`, {
    headers: { 'X-Api-Key': key },
  });
  if (!r.ok) throw Error(`${kind} HTTP ${r.status}`);
  return r.json();
}
const results = [];
try {
  await call('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  for (const kind of ['shows', 'music']) {
    const roots = (await call('/catalog/roots')).items;
    const root =
      roots.find(
        (r) => r.path === `/media/${kind === 'shows' ? 'tv' : kind}`,
      ) ??
      (await call('/catalog/roots', 'POST', {
        kind,
        name: `Acquisition ${kind}`,
        path: `/media/${kind === 'shows' ? 'tv' : kind}`,
      }));
    const job = (await call(`/catalog/roots/${root.id}/scan`, 'POST')).job_id;
    await expect
      .poll(
        async () =>
          (await call('/admin/jobs')).items.find((j) => j.id === job)?.state,
        { timeout: 60000 },
      )
      .toBe('complete');
  }
  await call('/admin/managers/reconcile', 'POST');
  const bindings = (await call('/admin/managers/bindings')).items;
  const tv = bindings.find((f) => f.path.startsWith('/media/tv/'));
  const music = bindings.find((f) => f.path.startsWith('/media/music/'));
  expect(tv?.ownership).toBe('managed');
  expect(music?.ownership).toBe('managed');
  const episodes = await manager('sonarr', 28989, 3, 'episode?seriesId=1');
  const expected = episodes.find(
    (e) => e.episodeNumber === 2 && e.seasonNumber === 1,
  );
  const untouched = episodes.find(
    (e) => e.episodeNumber === 1 && e.seasonNumber === 1,
  );
  expect(tv.path).toContain('S01E01');
  expect(tv.bindings[0].members).toEqual([expected.id]);
  const episode = (await call('/catalog?kind=episode')).items.find(
    (e) => e.available,
  );
  const album = (await call('/catalog?kind=album')).items.find(
    (e) => e.title === 'Discovery',
  );
  if (!episode || !album) throw Error('Missing generated catalog fixture');
  for (const [name, media] of [
    ['sonarr', episode.parent_id],
    ['lidarr', album.id],
  ]) {
    await expect
      .poll(
        async () => {
          const commands = await manager(
            name,
            name === 'sonarr' ? 28989 : 28686,
            name === 'sonarr' ? 3 : 1,
            'command',
          );
          return commands.filter((c) =>
            ['queued', 'started'].includes(c.status),
          ).length;
        },
        { timeout: 120000, intervals: [1000, 3000] },
      )
      .toBe(0);
    const operation = await call('/admin/media/operations', 'POST', {
      media_id: media,
      action: 'delete',
    });
    await call(`/admin/media/operations/${operation.id}/execute`, 'POST');
    results.push({ manager: name, deleted: true });
  }
  const after = await manager('sonarr', 28989, 3, 'episode?seriesId=1');
  expect(after.find((e) => e.id === expected.id).monitored).toBe(false);
  expect(after.find((e) => e.id === untouched.id).monitored).toBe(
    untouched.monitored,
  );
  expect(after.find((e) => e.id === expected.id).hasFile).toBe(false);
  writeFileSync(
    '.local/manager-tv-music-result.json',
    JSON.stringify({ results, provider_numbering_mismatch: true }, null, 2),
  );
  console.log(JSON.stringify({ results, provider_numbering_mismatch: true }));
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
} finally {
  await api.dispose();
}
