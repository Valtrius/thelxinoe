import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { resolve } from 'node:path';
import { randomBytes } from 'node:crypto';
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';

const image =
  'ghcr.io/recyclarr/recyclarr@sha256:6e69e009e1cd7493ff6093e8e187b5d3788c75b4a2c0c5127b6a1beda1c19728';
const project = `thelxinoe-recyclarr-qualification-${Date.now()}`;
const root = resolve(`.local/${project}`);
const key = randomBytes(16).toString('hex');
const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    maxBuffer: 8 * 1024 * 1024,
    timeout: 360000,
    killSignal: 'SIGKILL',
  }).trim();
const evidence = {
  image,
  project,
  root,
  started: new Date().toISOString(),
  finished: null,
  passed: null,
  assertions: [],
  commands: [],
  snapshots: {},
};
const fixtures = {
  radarr: {
    port: 7878,
    image:
      'lscr.io/linuxserver/radarr@sha256:c960f2b52ec6542dbe6707c5a21e696a7c74fd8b17997454f4d10a55dacee133',
    profile: 'd1d67249d3890e49bc12e275d989a7e9',
  },
  sonarr: {
    port: 8989,
    image:
      'lscr.io/linuxserver/sonarr@sha256:a5c1a5fecbef946927ab90ad68df319ac5fe644057e5fc18cd993f01ac07b2b2',
    profile: '72dae194fc92bf828f32cde7744e51a1',
  },
};
for (const dir of ['recyclarr', ...Object.keys(fixtures)])
  mkdirSync(`${root}/${dir}`, { recursive: true });
const ids = [];
async function api(kind, path) {
  const address = JSON.parse(docker('inspect', `${project}-${kind}`))[0]
    .NetworkSettings.Ports[`${fixtures[kind].port}/tcp`][0].HostPort;
  const response = await fetch(`http://127.0.0.1:${address}/api/v3/${path}`, {
    headers: { 'X-Api-Key': key },
    signal: AbortSignal.timeout(30000),
  });
  assert.ok(response.ok, `${kind}/${path}: ${response.status}`);
  return response.json();
}
async function snapshot(kind) {
  return {
    profiles: await api(kind, 'qualityprofile'),
    formats: await api(kind, 'customformat'),
    sizes: await api(kind, 'qualitydefinition'),
  };
}
function run(...args) {
  evidence.commands.push(args);
  const output = docker(
    'run',
    '--rm',
    '-t',
    '--user',
    '1000:1000',
    '--network',
    project,
    '-e',
    'TERM=dumb',
    '-v',
    `${root}/recyclarr:/config`,
    image,
    ...args,
  );
  assert.ok(!output.includes(key), 'CLI must redact API keys');
  writeFileSync(`${root}/command-${evidence.commands.length}.log`, output);
  return output;
}
try {
  docker('network', 'create', project);
  for (const [kind, spec] of Object.entries(fixtures)) {
    writeFileSync(
      `${root}/${kind}/config.xml`,
      `<Config><ApiKey>${key}</ApiKey><AuthenticationMethod>External</AuthenticationMethod><AuthenticationRequired>Enabled</AuthenticationRequired><Port>${spec.port}</Port><BindAddress>*</BindAddress><UrlBase></UrlBase></Config>`,
    );
    ids.push(
      docker(
        'run',
        '-d',
        '--name',
        `${project}-${kind}`,
        '--network',
        project,
        '--network-alias',
        kind,
        '-p',
        `127.0.0.1::${spec.port}`,
        '-v',
        `${root}/${kind}:/config`,
        '-e',
        'PUID=1000',
        '-e',
        'PGID=1000',
        spec.image,
      ),
    );
    let ready = false,
      last,
      initial,
      previous;
    for (const deadline = Date.now() + 90000; Date.now() < deadline;) {
      try {
        await api(kind, 'system/status');
        initial = await snapshot(kind);
        const current = JSON.stringify(initial);
        if (
          initial.profiles.length &&
          initial.sizes.length &&
          current === previous
        ) {
          ready = true;
          break;
        }
        previous = current;
        last = Error('Native quality settings are still initializing');
      } catch (error) {
        last = error;
      }
      await delay(1000);
    }
    assert.ok(ready, `${kind} startup: ${last}`);
    evidence.snapshots[`${kind}-before`] = initial;
  }
  for (const [kind, spec] of Object.entries(fixtures)) {
    const catalog = run('list', 'quality-profiles', kind, '--raw');
    assert.ok(catalog.includes(spec.profile), `${kind} curated guide exists`);
  }
  const configuration = (sizes = false) =>
    Object.entries(fixtures)
      .map(
        ([kind, spec]) =>
          `${kind}:\n  managed_${kind}:\n    base_url: http://${kind}:${spec.port}\n    api_key: !file /config/runtime-key\n    delete_old_custom_formats: false\n${sizes ? `    quality_definition:\n      type: ${kind === 'radarr' ? 'movie' : 'series'}\n` : ''}    quality_profiles:\n      - trash_id: ${spec.profile}\n        name: Thelxinoe ${kind} guide\n        reset_unmatched_scores:\n          enabled: false\n`,
      )
      .join('');
  writeFileSync(`${root}/recyclarr/recyclarr.yml`, configuration());
  writeFileSync(`${root}/recyclarr/runtime-key`, key);
  run('sync', '--preview', '--log', 'info');
  for (const kind of Object.keys(fixtures))
    assert.deepEqual(
      await snapshot(kind),
      evidence.snapshots[`${kind}-before`],
      `${kind} preview changes nothing`,
    );
  evidence.assertions.push(
    'Preview leaves every profile, custom format, and quality-size setting unchanged',
  );
  run('sync', '--log', 'info');
  for (const kind of Object.keys(fixtures)) {
    const applied = await snapshot(kind);
    evidence.snapshots[`${kind}-applied`] = applied;
    const profile = applied.profiles.find(
      (p) => p.name === `Thelxinoe ${kind} guide`,
    );
    assert.ok(
      profile?.upgradeAllowed && profile.formatItems.some((f) => f.score > 0),
      `${kind} profile and scores applied`,
    );
    assert.ok(applied.formats.length > 8, `${kind} custom formats applied`);
    assert.deepEqual(
      applied.sizes,
      evidence.snapshots[`${kind}-before`].sizes,
      'Quality-size settings are opt-in',
    );
  }
  run('sync', '--log', 'info');
  for (const kind of Object.keys(fixtures))
    assert.deepEqual(
      await snapshot(kind),
      evidence.snapshots[`${kind}-applied`],
      `${kind} repeated sync is idempotent`,
    );
  evidence.assertions.push(
    'Real CF definitions, quality profiles, and scores apply together; repeated sync is idempotent',
  );
  writeFileSync(`${root}/recyclarr/recyclarr.yml`, configuration(true));
  run('sync', '--log', 'info');
  for (const kind of Object.keys(fixtures)) {
    const guide = JSON.parse(
      readFileSync(
        `${root}/recyclarr/resources/trash-guides/git/official/docs/json/${kind}/quality-size/${kind === 'radarr' ? 'movie' : 'series'}.json`,
      ),
    );
    const quality = guide.qualities.find(
      (row) => row.quality === 'WEBDL-1080p',
    );
    let saved;
    for (let attempt = 0; attempt < 15; attempt++) {
      saved = (await api(kind, 'qualitydefinition')).find(
        (row) => row.quality.name === quality.quality,
      );
      if (saved.minSize === quality.min) break;
      await delay(1000);
    }
    assert.equal(
      saved.minSize,
      quality.min,
      `${kind} opted-in size limits match guide`,
    );
  }
  evidence.assertions.push(
    'Opted-in quality-size limits match the official guide in both native APIs',
  );
  const state = docker(
    'run',
    '--rm',
    '--entrypoint',
    'sh',
    '-v',
    `${root}/recyclarr:/config`,
    image,
    '-c',
    'find /config/state -type f; cat /config/resources/trash-guides/git/official/.git/HEAD; cat /config/resources/trash-guides/git/official/.git/refs/heads/master',
  );
  evidence.state = state;
  assert.ok(
    state.includes('quality-profile-mappings.json'),
    'Profile state is durable',
  );
  evidence.passed = true;
  console.log(`Qualified Recyclarr: ${root}`);
} catch (error) {
  evidence.passed = false;
  evidence.error = String(error).replaceAll(key, '[redacted]');
  throw error;
} finally {
  evidence.cleanup_errors = [];
  const cleanup = (name, action) => {
    try {
      action();
    } catch (error) {
      evidence.cleanup_errors.push(
        `${name}: ${String(error).replaceAll(key, '[redacted]')}`,
      );
    }
  };
  for (const container of ids)
    cleanup(container, () => docker('rm', '-f', container));
  cleanup('fixture network', () => docker('network', 'rm', project));
  cleanup('runtime key', () =>
    rmSync(`${root}/recyclarr/runtime-key`, { force: true }),
  );
  for (const kind of Object.keys(fixtures))
    cleanup(`${kind} key`, () =>
      rmSync(`${root}/${kind}/config.xml`, { force: true }),
    );
  evidence.finished = new Date().toISOString();
  if (evidence.cleanup_errors.length) {
    evidence.passed = false;
    process.exitCode = 1;
  }
  writeFileSync(`${root}/result.json`, JSON.stringify(evidence, null, 2));
  writeFileSync(
    '.local/recyclarr-qualification-result.json',
    JSON.stringify(evidence, null, 2),
  );
}
