import { request, chromium } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const origin = 'https://localhost:20443';
const api = await request.newContext({
  baseURL: origin,
  ignoreHTTPSErrors: true,
  extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
});
async function call(path, method = 'GET', data, client = api) {
  const response = await client.fetch(`/api/v1${path}`, { method, data });
  const body = await response.json();
  if (!response.ok())
    throw new Error(
      `${method} ${path.split('?')[0]} ${response.status()}: ${body.error?.message}`,
    );
  return body;
}
const caps = {
  containers: ['mp4', 'flac'],
  video: ['h264'],
  audio: ['aac', 'flac'],
  hls: true,
};
const options = {
  quality: 'auto',
  audio: null,
  subtitle: null,
  capabilities: caps,
};
let browser;
try {
  const credentials = {
    username: 'admin',
    password: 'test-only long passphrase',
  };
  if ((await call('/setup')).setup_required) {
    await call('/setup', 'POST', credentials);
  } else await call('/auth/login', 'POST', credentials);
  const roots = (await call('/catalog/roots')).items;
  for (const kind of ['movies', 'music']) {
    const root =
      roots.find((r) => r.kind === kind) ??
      (await call('/catalog/roots', 'POST', {
        kind,
        name: `Playback ${kind}`,
        path: `/media/playback/${kind}`,
      }));
    const job = (await call(`/catalog/roots/${root.id}/scan`, 'POST')).job_id;
    for (let i = 0; i < 120; i++) {
      const result = (await call('/admin/jobs')).items.find(
        (j) => j.id === job,
      );
      if (result?.state === 'complete') break;
      if (result?.state === 'failed') throw new Error(result.error);
      await new Promise((r) => setTimeout(r, 500));
    }
  }
  const items = (await call('/catalog?kind=movie')).items;
  const direct = items.find((i) => i.title === 'Direct'),
    remux = items.find((i) => i.title === 'Remux'),
    transcode = items.find((i) => i.title === 'Transcode');
  assert.ok(direct && remux && transcode);
  const play = async (item, quality = 'auto', client = api, file) =>
    call(
      '/playback',
      'POST',
      {
        media_id: item.id,
        file_id: file,
        position: 0,
        options: { ...options, quality },
      },
      client,
    );
  const first = await play(direct);
  assert.equal(first.mode, 'direct');
  const ranged = await api.get(first.url, {
    headers: { Range: 'bytes=0-1023' },
  });
  assert.equal(ranged.status(), 206);
  assert.equal((await ranged.body()).length, 1024);
  assert.match(ranged.headers()['content-range'], /^bytes 0-1023\//);
  const head = await api.head(first.url);
  assert.equal(head.status(), 200);
  assert.ok(Number(head.headers()['content-length']) > 1024);
  assert.equal(
    (
      await api.get(first.url, { headers: { Range: 'bytes=999999999999-' } })
    ).status(),
    416,
  );
  assert.ok(first.subtitles.length);
  const subtitle = await api.get(first.subtitles[0].url);
  assert.equal(subtitle.status(), 200);
  assert.match(
    await subtitle.text(),
    /WEBVTT[\s\S]*Thelxinoe subtitle fixture/,
  );
  browser = await chromium.launch({
    headless: true,
    args: ['--autoplay-policy=no-user-gesture-required'],
  });
  const context = await browser.newContext({
    ignoreHTTPSErrors: true,
    storageState: await api.storageState(),
  });
  await context.addInitScript(() => {
    window.__audioProbe = [];
    const connect = AudioNode.prototype.connect;
    AudioNode.prototype.connect = function (...args) {
      this.__target = args[0];
      return connect.apply(this, args);
    };
    const start = AudioBufferSourceNode.prototype.start;
    AudioBufferSourceNode.prototype.start = function (...args) {
      window.__audioProbe.push({
        source: this,
        when: args[0] ?? 0,
        offset: args[1] ?? 0,
        gain: this.__target?.gain?.value ?? 1,
      });
      return start.apply(this, args);
    };
  });
  const page = await context.newPage();
  await page.goto(origin);
  await page.getByRole('button', { name: 'Movies', exact: true }).click();
  await page.getByRole('button', { name: 'Direct 2020', exact: true }).click();
  await page.getByRole('button', { name: 'Play media', exact: true }).click();
  await page.waitForFunction(
    () => {
      const v = document.querySelector('video');
      return (
        v &&
        v.currentTime > 0.2 &&
        v.getVideoPlaybackQuality().totalVideoFrames > 0
      );
    },
    {},
    { timeout: 30000 },
  );
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  await page.screenshot({ path: '.local/playback-direct.png', fullPage: true });
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  console.log(
    'HTTPS direct range, browser decoding and sidecar subtitle checks passed.',
  );
  for (const [item, mode, quality] of [
    [remux, 'remux', 'auto'],
    [transcode, 'transcode', 'auto'],
    [direct, 'transcode', '2mbps'],
  ]) {
    const session = await play(item, quality);
    assert.equal(session.mode, mode);
    const playlist = await api.get(session.url);
    assert.equal(playlist.status(), 200);
    const body = await playlist.text();
    assert.match(body, /#EXTM3U/);
    const segment = body.split('\n').find((s) => s.startsWith('segment-'));
    assert.ok(segment);
    const data = await api.get(
      new URL(segment, new URL(session.url, origin)).toString(),
    );
    assert.equal(data.status(), 200);
    assert.ok((await data.body()).length > 1000);
    await call(`/playback/${session.id}/progress`, 'POST', {
      sequence: 0,
      position: 2,
      state: 'playing',
    });
    const seek = await call(`/playback/${session.id}/seek`, 'POST', {
      position: 12,
    });
    assert.equal(seek.timeline_start, 12);
    assert.equal((await api.get(seek.url)).status(), 200);
    await call(`/playback/${session.id}/progress`, 'POST', {
      sequence: 1,
      position: 13,
      state: 'stopped',
    });
    assert.ok([401, 404].includes((await api.get(seek.url)).status()));
    console.log(
      `HTTPS ${mode} (${quality}), segment delivery, seek and stop checks passed.`,
    );
  }
  for (const title of ['Remux', 'Transcode']) {
    await page
      .getByRole('button', { name: `${title} 2020`, exact: true })
      .click();
    await page.getByRole('button', { name: 'Play media', exact: true }).click();
    await page.waitForFunction(
      () => {
        const v = document.querySelector('video');
        return (
          v &&
          v.currentTime > 0.2 &&
          v.getVideoPlaybackQuality().totalVideoFrames > 0
        );
      },
      {},
      { timeout: 30000 },
    );
    await page
      .getByLabel('Playback position', { exact: true })
      .evaluate((input) => {
        input.value = '12';
      });
    await page
      .getByLabel('Playback position', { exact: true })
      .dispatchEvent('change');
    await page.waitForFunction(
      () => {
        const v = document.querySelector('video');
        return (
          v &&
          !v.paused &&
          v.currentTime > 0.2 &&
          Number(
            document.querySelector('[aria-label="Playback position"]').value,
          ) >= 12
        );
      },
      {},
      { timeout: 30000 },
    );
    await page
      .getByRole('button', { name: 'Close player', exact: true })
      .click();
  }
  console.log('Real Chromium HLS remux/transcode decoding and seeks passed.');
  await page.getByRole('button', { name: 'Music', exact: true }).click();
  await page
    .getByRole('button', { name: 'Gapless Artist artist', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Gapless Album album', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Gapless 1 track', exact: true })
    .click();
  await page.getByRole('button', { name: 'Play media', exact: true }).click();
  await page
    .getByRole('heading', { name: 'Queue finished', exact: true })
    .waitFor({ timeout: 20000 });
  const audioProof = await page.evaluate(async () => {
    const scheduled = window.__audioProbe.slice(0, 2);
    if (scheduled.length !== 2)
      throw new Error(
        `Expected two scheduled buffers, saw ${scheduled.length}`,
      );
    const [first, second] = scheduled;
    const rate = first.source.buffer.sampleRate;
    const duration = first.source.buffer.duration - first.offset;
    const offline = new OfflineAudioContext(
      1,
      Math.round((duration + second.source.buffer.duration) * rate),
      rate,
    );
    for (const entry of scheduled) {
      const node = offline.createBufferSource();
      node.buffer = entry.source.buffer;
      const gain = offline.createGain();
      gain.gain.value = entry.gain;
      node.connect(gain).connect(offline.destination);
      node.start(entry.when - first.when, entry.offset);
    }
    const rendered = await offline.startRendering();
    const samples = rendered.getChannelData(0);
    const boundary = Math.round(duration * rate);
    let min = 1,
      max = 0;
    for (let i = boundary - 64; i < boundary + 64; i++) {
      min = Math.min(min, samples[i]);
      max = Math.max(max, samples[i]);
    }
    return {
      gap: second.when - first.when - duration,
      rate,
      min,
      max,
      gain: first.gain,
    };
  });
  assert.ok(Math.abs(audioProof.gap) < 1 / audioProof.rate);
  assert.ok(Math.abs(audioProof.gain - 0.5) < 0.00001);
  assert.ok(audioProof.min > 0.249 && audioProof.max < 0.251);
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  console.log(
    'Real browser FLAC sequencing has a sample-continuous seam with ReplayGain applied (-6.02 dB).',
  );
  const name = `viewer-${Date.now()}`;
  await call('/users', 'POST', {
    username: name,
    password: 'test-only viewer passphrase',
    role: 'user',
  });
  const viewer = await request.newContext({
    baseURL: origin,
    ignoreHTTPSErrors: true,
    extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
  });
  try {
    await call(
      '/auth/login',
      'POST',
      { username: name, password: 'test-only viewer passphrase' },
      viewer,
    );
    const second = await play(direct, 'auto', viewer);
    await call(`/playback/${first.id}/progress`, 'POST', {
      sequence: 2,
      position: 17,
      state: 'paused',
    });
    await call(`/playback/${first.id}/progress`, 'POST', {
      sequence: 1,
      position: 1,
      state: 'playing',
    });
    await call(
      `/playback/${second.id}/progress`,
      'POST',
      { sequence: 0, position: 4, state: 'playing' },
      viewer,
    );
    const own = await call(`/catalog/${direct.id}/playback`),
      other = await call(
        `/catalog/${direct.id}/playback`,
        'GET',
        undefined,
        viewer,
      );
    assert.equal(own.watched, true);
    assert.equal(other.watched, false);
    assert.equal(own.progress[0].position, 17);
    assert.equal(other.progress[0].position, 4);
    assert.equal(
      (
        await viewer.post(`/api/v1/playback/${first.id}/progress`, {
          data: { sequence: 9, position: 2, state: 'playing' },
        })
      ).status(),
      404,
    );
    const sessions = await call('/auth/sessions', 'GET', undefined, viewer);
    await call(
      `/auth/sessions/${sessions.current}`,
      'DELETE',
      undefined,
      viewer,
    );
    assert.equal((await api.get(second.url)).status(), 401);
  } finally {
    await viewer.dispose();
  }
  await call(`/playback/${first.id}/progress`, 'POST', {
    sequence: 3,
    position: 17,
    state: 'stopped',
  });
  const edition = items.find((i) => i.title === 'Edition');
  assert.ok(edition);
  const editions = (await call(`/catalog/${edition.id}/playback`)).sources;
  assert.equal(editions.length, 2);
  for (const [index, file] of editions.entries()) {
    const session = await play(edition, 'auto', api, file.id);
    await call(`/playback/${session.id}/progress`, 'POST', {
      sequence: 0,
      position: 3 + index * 4,
      state: 'stopped',
    });
  }
  const saved = (await call(`/catalog/${edition.id}/playback`)).progress;
  assert.equal(saved.length, 2);
  assert.deepEqual(saved.map((p) => p.position).sort(), [3, 7]);
  console.log(
    'Independent users, out-of-order progress, 90% watched inference, device revocation and edition resume checks passed.',
  );
  writeFileSync(
    '.local/playback-result.json',
    JSON.stringify(
      {
        verified_at: new Date().toISOString(),
        range: true,
        direct_browser: true,
        hls_remux: true,
        hls_transcode: true,
        seek: true,
        subtitles: true,
        multi_user: true,
        edition_resume: true,
        hls_browser_decode: true,
        gapless_sample_seam: true,
        replay_gain: true,
      },
      null,
      2,
    ),
  );
} finally {
  await browser?.close();
  await api.dispose();
}
