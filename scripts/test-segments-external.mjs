// Real provider metadata attached explicitly to generated, non-copyright media.
import { request, expect } from '@playwright/test';
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { execFileSync } from 'node:child_process';
const root = resolve(
  '.local/segments/data/external/The Walking Dead/Season 01',
);
mkdirSync(root, { recursive: true });
const file = 'The Walking Dead - S01E01.mp4';
if (!existsSync(join(root, file))) {
  try {
    execFileSync(
      'docker',
      [
        'run',
        '--rm',
        '--network',
        'none',
        '--entrypoint',
        'ffmpeg',
        '-v',
        `${root}:/fixture:rw`,
        'thelxinoe-server:0.1.0',
        '-nostdin',
        '-hide_banner',
        '-loglevel',
        'error',
        '-f',
        'lavfi',
        '-i',
        'color=c=navy:s=320x180:r=24',
        '-f',
        'lavfi',
        '-i',
        'anullsrc=r=48000:cl=stereo',
        '-t',
        '420',
        '-c:v',
        'libx264',
        '-preset',
        'ultrafast',
        '-c:a',
        'aac',
        '-movflags',
        '+faststart',
        `/fixture/${file}`,
      ],
      { stdio: 'pipe', windowsHide: true },
    );
  } catch {
    throw Error('External timestamp fixture encoding failed');
  }
}
const c = await request.newContext({
  baseURL: 'https://localhost:26443',
  ignoreHTTPSErrors: true,
  extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
});
async function api(path, method = 'GET', data) {
  const r = await c.fetch('/api/v1' + path, { method, data, timeout: 120000 });
  const body = await r.json();
  if (!r.ok())
    throw Error(`${path}: HTTP ${r.status()} ${body.error?.message ?? ''}`);
  return body;
}
async function job(id) {
  await expect
    .poll(
      async () => {
        const row = (await api('/admin/jobs')).items.find((j) => j.id === id);
        if (row?.state === 'failed') throw Error(row.error);
        return row?.state;
      },
      { timeout: 180000, intervals: [2000] },
    )
    .toBe('complete');
}
let original;
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  original = (await api('/admin/segments')).config;
  await api('/admin/metadata', 'PUT', {
    tmdb_token: readFileSync('.local/tmdb-token', 'utf8').trim(),
    musicbrainz_contact: readFileSync(
      '.local/musicbrainz-contact',
      'utf8',
    ).trim(),
  });
  const roots = (await api('/catalog/roots')).items;
  const library =
    roots.find((r) => r.path === '/media/external') ??
    (await api('/catalog/roots', 'POST', {
      name: 'External timestamp fixture',
      kind: 'shows',
      path: '/media/external',
    }));
  await job((await api(`/catalog/roots/${library.id}/scan`, 'POST')).job_id);
  const show = (await api('/catalog?kind=show')).items.find(
    (s) => s.title === 'The Walking Dead',
  );
  expect(show).toBeTruthy();
  await job(
    (
      await api(`/catalog/${show.id}/match`, 'POST', {
        provider: 'tmdb',
        external_id: '1402',
      })
    ).job_id,
  );
  const provider = (
    await api(`/catalog/${show.id}/provider-episodes`)
  ).items.find((e) => e.season === 1 && e.episode === 1);
  expect(provider).toBeTruthy();
  const season = (await api(`/catalog?parent=${show.id}`)).items.find(
    (s) => s.sort_number === 1,
  );
  expect(season).toBeTruthy();
  const episode = (await api(`/catalog?parent=${season.id}`)).items.find(
    (e) => e.sort_number === 1,
  );
  expect(episode).toBeTruthy();
  await api(`/catalog/${episode.id}/episode-mapping`, 'PUT', {
    episode_ids: [provider.id],
  });
  await api('/admin/segments', 'PUT', { local: false, external: true });
  await api(`/catalog/${episode.id}/segments/analyze`, 'POST');
  await expect
    .poll(
      async () =>
        (await api('/admin/segments')).items.find(
          (r) => r.media_id === episode.id,
        )?.state,
      { timeout: 60000, intervals: [1000] },
    )
    .toBe('complete');
  const segments = await api(`/catalog/${episode.id}/segments`);
  const intro = segments.items.find((s) => s.kind === 'Intro');
  expect(intro?.source).toBe('theintrodb');
  expect(intro.start).toBe(263);
  expect(intro.end).toBe(298.6);
  writeFileSync(
    '.local/segments-external-result.json',
    JSON.stringify(
      {
        passed: true,
        provider: 'TheIntroDB',
        series: 1402,
        season: 1,
        episode: 1,
        segments: segments.items,
      },
      null,
      2,
    ),
  );
  console.log(
    'Real TMDB episode mapping and TheIntroDB timestamp import passed',
  );
} finally {
  if (original) await api('/admin/segments', 'PUT', original);
  await c.dispose();
}
