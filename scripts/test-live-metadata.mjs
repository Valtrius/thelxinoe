import { chromium, request } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';

// Explicit opt-in: uses private local configuration and real provider requests.
const token = readFileSync('.local/tmdb-token', 'utf8').trim();
const contact = readFileSync('.local/musicbrainz-contact', 'utf8').trim();
const origin = 'https://localhost:19443';
const api = await request.newContext({
  baseURL: origin,
  ignoreHTTPSErrors: true,
  extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
});
async function call(path, method = 'GET', data) {
  const response = await api.fetch(`/api/v1${path}`, { method, data });
  const body = await response.json();
  if (!response.ok())
    throw new Error(
      `${method} ${path.split('?')[0]} returned ${response.status()}: ${body.error?.message ?? 'Request failed'}`,
    );
  return body;
}
async function finished(id) {
  const deadline = Date.now() + 120000;
  while (Date.now() < deadline) {
    const job = (await call('/admin/jobs')).items.find((job) => job.id === id);
    if (job?.state === 'complete') return;
    if (job?.state === 'failed')
      throw new Error(`Provider job failed: ${job.error}`);
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error('Provider job timed out');
}
async function match(item, candidate) {
  await finished(
    (
      await call(`/catalog/${item.id}/match`, 'POST', {
        provider: candidate.provider,
        external_id: candidate.id,
      })
    ).job_id,
  );
  return await call(`/catalog/${item.id}`);
}
try {
  const setup = await call('/setup');
  const credentials = {
    username: 'admin',
    password: 'test-only long passphrase',
  };
  if (setup.setup_required) {
    const code = execFileSync(
      'docker',
      [
        'compose',
        '-p',
        'thelxinoe-live',
        '-f',
        'compose.test.yaml',
        'exec',
        '-T',
        'server',
        'cat',
        '/var/lib/thelxinoe/secrets/setup-token',
      ],
      { encoding: 'utf8', windowsHide: true },
    ).trim();
    await call('/setup', 'POST', { ...credentials, setup_token: code });
  } else await call('/auth/login', 'POST', credentials);
  await call('/admin/metadata', 'PUT', {
    tmdb_token: token,
    musicbrainz_contact: contact,
  });
  const config = await call('/admin/metadata');
  assert.equal(config.tmdb_configured, true);
  assert.equal(JSON.stringify(config).includes(token), false);
  const roots = (await call('/catalog/roots')).items;
  for (const kind of ['movies', 'shows', 'music']) {
    const root =
      roots.find((root) => root.kind === kind) ??
      (await call('/catalog/roots', 'POST', {
        kind,
        name: `Live ${kind}`,
        path: `/data/${kind}`,
      }));
    await finished(
      (await call(`/catalog/roots/${root.id}/scan`, 'POST')).job_id,
    );
  }
  const items = (await call('/catalog')).items;
  const movie = items.find((item) => item.kind === 'movie' && item.available);
  const show = items.find((item) => item.kind === 'show');
  const artist = items.find((item) => item.kind === 'artist');
  const album = items.find((item) => item.kind === 'album');
  const candidates = async (kind, q) =>
    (await call(`/metadata/search?kind=${kind}&q=${encodeURIComponent(q)}`))
      .items;
  const film = (await candidates('movie', 'The Matrix')).find(
    (v) => v.title === 'The Matrix' && v.year === '1999',
  );
  assert.ok(film, 'TMDB must find the 1999 film');
  await match(movie, film);
  assert.ok((await call('/catalog/collections')).items.length);
  await call(`/catalog/${movie.id}/overrides`, 'PUT', {
    title: 'My Matrix correction',
  });
  await finished((await call(`/catalog/${movie.id}/refresh`, 'POST')).job_id);
  assert.equal(
    (await call(`/catalog/${movie.id}`)).title,
    'My Matrix correction',
  );
  const television = (await candidates('show', 'Firefly')).find(
    (v) => v.title === 'Firefly' && v.year === '2002',
  );
  assert.ok(television);
  await match(show, television);
  const providerEpisodes = (await call(`/catalog/${show.id}/provider-episodes`))
    .items;
  const episode = items.find((item) => item.kind === 'episode');
  await call(`/catalog/${episode.id}/episode-mapping`, 'PUT', {
    episode_ids: [],
  });
  assert.equal(
    (await call(`/catalog/${episode.id}/provider-episodes`)).mappings.length,
    0,
  );
  await call(`/catalog/${episode.id}/episode-mapping`, 'PUT', {
    episode_ids: [providerEpisodes.find((e) => e.season === 1).id],
  });
  assert.equal(
    (await call(`/catalog/${episode.id}/provider-episodes`)).mappings[0].state,
    'confirmed',
  );
  const musician = (await candidates('artist', 'artist:"Daft Punk"')).find(
    (v) => v.title === 'Daft Punk',
  );
  assert.ok(musician);
  await match(artist, musician);
  const albums = await candidates(
    'album',
    'release:"Discovery" AND artist:"Daft Punk"',
  );
  let cover = false;
  for (const candidate of albums
    .filter((v) => v.title === 'Discovery')
    .slice(0, 4)) {
    cover = (await match(album, candidate)).metadata.artwork_cached === true;
    if (cover) break;
  }
  assert.ok(cover, 'CAA must provide a real album cover');
  const updated = (await call('/catalog')).items;
  for (const item of [movie, show, album]) {
    const imageUrl = updated.find((v) => v.id === item.id).artwork_url;
    assert.ok(imageUrl, 'Matched media must expose cached artwork');
    const image = await api.get(imageUrl);
    assert.equal(image.status(), 200);
    assert.match(image.headers()['content-type'], /^image\/(jpeg|png)/);
    assert.ok((await image.body()).length > 1000);
  }
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({
      ignoreHTTPSErrors: true,
      storageState: await api.storageState(),
    });
    await page.goto(origin);
    await page.getByRole('button', { name: 'Movies', exact: true }).click();
    await page
      .getByRole('button', { name: 'My Matrix correction 1999', exact: true })
      .waitFor();
    await page.screenshot({ path: '.local/live-metadata.png', fullPage: true });
  } finally {
    await browser.close();
  }
  writeFileSync(
    '.local/live-metadata-result.json',
    JSON.stringify(
      {
        verified_at: new Date().toISOString(),
        movie: 'The Matrix (1999)',
        show: 'Firefly (2002)',
        artist: 'Daft Punk',
        album: 'Discovery',
        artwork: true,
        manual_override_survives_refresh: true,
        explicit_episode_mapping: true,
      },
      null,
      2,
    ),
  );
  console.log(
    'Live TMDB matching, collection, TV mapping, MusicBrainz, CAA artwork and manual correction refresh passed.',
  );
} finally {
  await api.dispose();
}
