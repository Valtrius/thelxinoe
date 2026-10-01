import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fixtureId, resourceRecord } from './ci-resources.mjs';
import { budgets, waitForState } from './ci-readiness.mjs';

const image =
  process.env.THELXINOE_TOOLS_TEST_IMAGE ?? 'thelxinoe-server:tools';
const name = fixtureId('tools');
const resources = resourceRecord({
  project: name,
  networks: [name],
  volumes: [name, `${name}-restored`],
  closed: false,
});
const output = resolve('test-results/server-tools');
mkdirSync(output, { recursive: true });
const catalog = JSON.parse(
  readFileSync('scripts/server-tools.lock.json', 'utf8'),
);
const catalogFile = resolve(output, `${name}-catalog.json`);
const evidence = {
  command: `pnpm run test:server-tools ${process.argv.slice(2).join(' ')}`,
  image,
  started: new Date().toISOString(),
  results: [],
  packages: catalog,
  product_passed: false,
  cleanup_errors: [],
};
let token = '';
let base = '';
function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value && typeof value === 'object')
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
      .join(',')}}`;
  return JSON.stringify(value);
}
function identify(candidate) {
  const value = { ...candidate };
  delete value.id;
  return createHash('sha256').update(canonical(value)).digest('hex');
}
async function docker(...args) {
  const labels = [
    '--label',
    `io.thelxinoe.ci-run=${process.env.THELXINOE_CI_RUN_ID}`,
    '--label',
    `com.docker.compose.project=${name}`,
  ];
  if (args[0] === 'run') args.splice(1, 0, ...labels);
  else if (['network', 'volume'].includes(args[0]) && args[1] === 'create')
    args.splice(2, 0, ...labels);
  return new Promise((accept, reject) => {
    const child = spawn('docker', args, { windowsHide: true });
    let out = '',
      error = '';
    child.stdout.on('data', (data) => (out += data));
    child.stderr.on('data', (data) => (error += data));
    child.on('error', reject);
    child.on('close', (code) =>
      code === 0
        ? accept(out.trim())
        : reject(new Error(`docker ${args[0]} failed: ${error.slice(-5000)}`)),
    );
  });
}
function eventually(callback, timeout = budgets.provision) {
  return waitForState('Server tool state', callback, (value) => !!value, {
    timeout,
  });
}
async function api(
  path,
  method = 'GET',
  body,
  expected = 200,
  credential = token,
) {
  const response = await fetch(`${base}/api/v1${path}`, {
    method,
    signal: AbortSignal.timeout(10000),
    headers: {
      'Content-Type': 'application/json',
      'X-Thelxinoe-Client': '1',
      ...(credential ? { Authorization: `Bearer ${credential}` } : {}),
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const value = await response.json();
  assert.equal(
    response.status,
    expected,
    `${method} ${path}: ${JSON.stringify(value)}`,
  );
  return value;
}
async function inventory() {
  return (await api('/admin/tools')).items;
}
async function idle() {
  return eventually(async () => {
    const items = await inventory();
    return (
      items.every(
        (item) =>
          !item.job ||
          ['complete', 'failed', 'canceled'].includes(item.job.stage),
      ) && items
    );
  });
}
async function saveCatalog() {
  writeFileSync(catalogFile, JSON.stringify(catalog, null, 2));
  await docker('cp', catalogFile, `${name}:/var/lib/thelxinoe/catalog.json`);
  await docker(
    'exec',
    '--user',
    '0',
    name,
    'chown',
    '10001:10001',
    '/var/lib/thelxinoe/catalog.json',
  );
}
async function check() {
  await idle();
  await api('/admin/tools/check', 'POST', {});
  return idle();
}
async function scenario(title, run) {
  const started = Date.now();
  try {
    const details = await run();
    evidence.results.push({
      title,
      passed: true,
      duration_ms: Date.now() - started,
      details,
    });
    console.log(`PASS ${title}`);
  } catch (error) {
    evidence.results.push({ title, passed: false, error: String(error) });
    throw error;
  } finally {
    save();
  }
}
function save() {
  writeFileSync(
    resolve(output, 'results.json'),
    JSON.stringify(evidence, null, 2),
  );
  const escape = (value) =>
    String(value)
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;');
  const status = evidence.finished
    ? evidence.passed
      ? 'Passed'
      : 'Failed'
    : 'Running';
  writeFileSync(
    resolve(output, 'index.html'),
    `<!doctype html><meta charset="utf-8"><title>Server tool qualification</title><h1>Server tool qualification: ${status}</h1><p>${escape(evidence.command)}</p><table><thead><tr><th>Scenario</th><th>Result</th></tr></thead><tbody>${evidence.results.map((result) => `<tr><td>${escape(result.title)}</td><td>${result.passed ? 'Passed' : escape(result.error)}</td></tr>`).join('')}</tbody></table>${evidence.cleanup_errors.length ? `<h2>Cleanup failed</h2><ul>${evidence.cleanup_errors.map((error) => `<li>${escape(error)}</li>`).join('')}</ul>` : ''}<p>Exact package digests, dependency locks and observations: <a href="results.json">results.json</a>. Live providers are not part of this deterministic run.</p>`,
  );
}
async function coldStart() {
  const container = `${name}-cold`;
  const root = resolve('.local', container);
  const coldResources = resourceRecord({
    project: container,
    containers: [container, `${container}-validate`],
    bindRoot: root,
    cleanupImage: image,
    closed: false,
  });
  for (const directory of ['state', 'cache'])
    mkdirSync(resolve(root, directory), { recursive: true });
  const transitions = [];
  try {
    await docker(
      'run',
      '--rm',
      '--network',
      'none',
      '--user',
      '0:0',
      '-v',
      `${root}:/fixture`,
      '--entrypoint',
      'chown',
      image,
      '-R',
      '10001:10001',
      '/fixture',
    );
    const started = Date.now();
    await docker(
      'run',
      '-d',
      '--name',
      container,
      '--health-interval',
      '1s',
      '--network',
      'none',
      '--read-only',
      '--cap-drop',
      'ALL',
      '--security-opt',
      'no-new-privileges:true',
      '--tmpfs',
      '/tmp:rw,noexec,nosuid,size=64m',
      '-v',
      `${resolve(root, 'state')}:/var/lib/thelxinoe`,
      '-v',
      `${resolve(root, 'cache')}:/var/cache/thelxinoe`,
      '-e',
      'THELXINOE_TOOLS_CATALOG=/opt/thelxinoe/tools/catalog.json',
      image,
    );
    for (;;) {
      const state = JSON.parse(
        await docker('inspect', '--format', '{{json .State}}', container),
      );
      if (transitions.at(-1)?.status !== state.Health?.Status)
        transitions.push({
          status: state.Health?.Status,
          elapsed_ms: Date.now() - started,
        });
      assert.equal(state.Running, true, JSON.stringify(state));
      assert.notEqual(
        state.Health?.Status,
        'unhealthy',
        JSON.stringify(state.Health),
      );
      if (state.Health?.Status === 'healthy') break;
      assert.ok(
        Date.now() - started < 30000,
        'Tool installation blocked HTTP startup',
      );
      await new Promise((accept) => setTimeout(accept, 1000));
    }
    const credential = JSON.parse(
      await docker(
        'exec',
        container,
        'curl',
        '-fsS',
        '-H',
        'Content-Type: application/json',
        '-H',
        'X-Thelxinoe-Client: 1',
        '-d',
        JSON.stringify({
          username: 'admin',
          password: 'cold startup test passphrase',
          transport: 'device',
        }),
        'http://127.0.0.1:8484/api/v1/setup',
      ),
    ).token;
    const readyMs = Date.now() - started;
    const readTools = async () =>
      JSON.parse(
        await docker(
          'exec',
          container,
          'curl',
          '-fsS',
          '-H',
          `Authorization: Bearer ${credential}`,
          'http://127.0.0.1:8484/api/v1/admin/tools',
        ),
      ).items;
    const pending = (item) =>
      item.job && !['complete', 'failed', 'canceled'].includes(item.job.stage);
    const observations = [];
    const observe = async () => {
      const items = await readTools();
      observations.push({ elapsed_ms: Date.now() - started, items });
      writeFileSync(
        resolve(output, 'cold-start-tools.json'),
        JSON.stringify(observations, null, 2),
      );
      return items;
    };
    const initial = await observe();
    assert.equal(initial.length, 4);
    assert.ok(
      initial.some((item) => !item.installed && pending(item)),
      'Missing tools must be shown as queued or installing while HTTP is ready',
    );
    for (const item of initial.filter((item) => !item.installed))
      assert.ok(pending(item), item.id);
    const queued = initial.find((item) => item.job?.stage === 'queued');
    assert.ok(queued, 'First-time installations must have a visible queue');
    await docker('restart', container);
    const recovered = await eventually(observe, 30000);
    assert.equal(
      recovered.find((item) => item.id === queued.id).job.id,
      queued.job.id,
      'Restart must resume the durable installation job',
    );
    const items = await eventually(async () => {
      const items = await observe();
      return (
        items.every(
          (item) => item.installed && !item.integrity_error && !pending(item),
        ) && items
      );
    });
    const toolsReadyMs = Date.now() - started;
    for (const item of items) {
      assert.ok(item.installed, item.id);
      assert.equal(item.integrity_error, null, item.id);
    }
    await docker('stop', container);
    const validationStarted = Date.now();
    const validationLog = await docker(
      'run',
      '--rm',
      '--name',
      `${container}-validate`,
      '--no-healthcheck',
      '--network',
      'none',
      '--read-only',
      '--cap-drop',
      'ALL',
      '--security-opt',
      'no-new-privileges:true',
      '--memory',
      '1g',
      '--cpus',
      '2',
      '--pids-limit',
      '64',
      '--tmpfs',
      '/tmp:rw,noexec,nosuid,size=64m',
      '--tmpfs',
      '/var/cache/thelxinoe:rw,nosuid,size=64m,uid=10001,gid=10001',
      '-v',
      `${resolve(root, 'state')}:/var/lib/thelxinoe`,
      image,
      'validate-state',
    );
    const validationMs = Date.now() - validationStarted;
    writeFileSync(resolve(output, 'cold-state-validation.log'), validationLog);
    const validation = JSON.parse(
      await docker(
        'run',
        '--rm',
        '--network',
        'none',
        '--read-only',
        '-v',
        `${resolve(root, 'state')}:/var/lib/thelxinoe:ro`,
        '--entrypoint',
        'cat',
        image,
        '/var/lib/thelxinoe/.release-validation.json',
      ),
    );
    writeFileSync(
      resolve(output, 'cold-state-validation.json'),
      JSON.stringify({ duration_ms: validationMs, validation }, null, 2),
    );
    assert.ok(
      validationMs < 180000,
      'Offline validation exceeded the release worker budget',
    );
    return {
      transitions,
      ready_ms: readyMs,
      tools_ready_ms: toolsReadyMs,
      items,
      validation_ms: validationMs,
      validation,
    };
  } finally {
    writeFileSync(
      resolve(output, 'cold-start-health.json'),
      JSON.stringify(transitions, null, 2),
    );
    await docker('logs', container)
      .then((logs) => writeFileSync(resolve(output, 'cold-start.log'), logs))
      .catch(() => {});
    try {
      coldResources.close();
    } catch (error) {
      evidence.cleanup_errors.push(error.message);
    }
  }
}
async function qualify() {
  try {
    if (!process.argv.includes('--built'))
      await docker('build', '--target', 'server', '-t', image, '.');
    evidence.image_id = await docker(
      'image',
      'inspect',
      '--format',
      '{{.Id}}',
      image,
    );
    await scenario(
      'HTTP starts before queued installations, restart resumes them, and offline release validation verifies all tools',
      coldStart,
    );
    if (process.argv.includes('--startup-only')) {
      evidence.product_passed = true;
      return;
    }
    await docker('network', 'create', '--internal', name);
    await docker('volume', 'create', name);
    await docker(
      'run',
      '-d',
      '--name',
      name,
      '--network',
      name,
      '-v',
      `${name}:/var/lib/thelxinoe`,
      '-e',
      'THELXINOE_TOOLS_CATALOG=/var/lib/thelxinoe/catalog.json',
      image,
    );
    // Docker's internal networks do not publish ports. A fixed-destination proxy
    // exposes only this server API; the server still has no route to the internet.
    const gateway = `import http.server,http.client
class Proxy(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  try:
   c=http.client.HTTPConnection('${name}',8484,timeout=30)
   data=self.rfile.read(int(self.headers.get('Content-Length',0)))
   c.request(self.command,self.path,body=data,headers=dict(self.headers))
   r=c.getresponse(); body=r.read(); self.send_response(r.status)
   self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(body))); self.end_headers(); self.wfile.write(body); c.close()
  except Exception:
   self.send_error(502)
 do_POST=do_GET
 do_PUT=do_GET
 do_DELETE=do_GET
 def log_message(self,*args): pass
http.server.ThreadingHTTPServer(('0.0.0.0',8080),Proxy).serve_forever()`;
    await docker(
      'run',
      '-d',
      '--name',
      `${name}-gateway`,
      '-p',
      '127.0.0.1::8080',
      '--entrypoint',
      'python3',
      image,
      '-c',
      gateway,
    );
    await docker('network', 'connect', name, `${name}-gateway`);
    const inspect = JSON.parse(await docker('inspect', `${name}-gateway`))[0];
    base = `http://127.0.0.1:${inspect.NetworkSettings.Ports['8080/tcp'][0].HostPort}`;
    await eventually(() => api('/health'), 300000);
    token = (
      await api('/setup', 'POST', {
        username: 'admin',
        password: 'server tools qualification passphrase',
        transport: 'device',
      })
    ).token;
    await saveCatalog();
    await scenario(
      'offline first boot imports four verified seeds',
      async () => {
        const items = await check();
        assert.equal(items.length, 4);
        for (const item of items) {
          assert.ok(item.installed, item.id);
          assert.equal(item.integrity_error, null, item.id);
        }
        return items;
      },
    );
    await scenario('all tool routes require authentication', async () => {
      await api('/admin/tools', 'GET', undefined, 401, '');
      await api('/admin/tools/check', 'POST', {}, 401, '');
      await api('/admin/tools/yt-dlp/check', 'POST', {}, 401, '');
      await api('/admin/tools/yt-dlp/install', 'POST', {}, 401, '');
      await api(
        '/admin/tools/yt-dlp/settings',
        'POST',
        { policy: 'notify', channel: 'nightly', pinned: false },
        401,
        '',
      );
      await api('/users', 'POST', {
        username: 'member',
        password: 'qualification member password',
        role: 'user',
      });
      const member = (
        await api('/auth/login', 'POST', {
          username: 'member',
          password: 'qualification member password',
          transport: 'device',
        })
      ).token;
      for (const [path, method, body] of [
        ['/admin/tools', 'GET'],
        ['/admin/tools/check', 'POST', {}],
        ['/admin/tools/yt-dlp/check', 'POST', {}],
        ['/admin/tools/yt-dlp/install', 'POST', {}],
        ['/admin/tools/yt-dlp/repair', 'POST', {}],
        ['/admin/tools/yt-dlp/rollback', 'POST', {}],
        [
          '/admin/tools/yt-dlp/settings',
          'POST',
          { policy: 'notify', channel: 'nightly', pinned: false },
        ],
      ]) {
        await api(path, method, body, 403, member);
      }
    });
    let original, updated;
    await scenario(
      'Notify discovers a new build without installing; manual install deduplicates',
      async () => {
        original = (await inventory()).find(
          (item) => item.id === 'yt-dlp',
        ).installed;
        catalog[0].source += '#qualification-build';
        catalog[0].id = identify(catalog[0]);
        await saveCatalog();
        const found = (await check()).find((item) => item.id === 'yt-dlp');
        assert.equal(found.installed.id, original.id);
        assert.equal(found.candidate.id, catalog[0].id);
        const body = { candidate_id: found.candidate.id };
        const [first, second] = await Promise.all([
          api('/admin/tools/yt-dlp/install', 'POST', body),
          api('/admin/tools/yt-dlp/install', 'POST', body),
        ]);
        assert.equal(first.job_id, second.job_id);
        updated = (await idle()).find((item) => item.id === 'yt-dlp');
        assert.equal(updated.job.stage, 'complete', updated.job.error);
        assert.notEqual(updated.installed.id, original.id);
        assert.equal(updated.previous.id, original.id);
        return updated;
      },
    );
    await scenario(
      'rollback retains exact previous package and holds rejected candidate',
      async () => {
        await api('/admin/tools/yt-dlp/rollback', 'POST', {
          candidate_id: original.candidate_id,
        });
        const item = (await idle()).find((value) => value.id === 'yt-dlp');
        assert.equal(item.installed.candidate_id, original.candidate_id);
        assert.equal(item.held, catalog[0].id);
        return item;
      },
    );
    await scenario(
      'single-tool checks work while pinned and leave other tools untouched',
      async () => {
        await api('/admin/tools/yt-dlp/settings', 'POST', {
          policy: 'notify',
          channel: 'nightly',
          pinned: true,
        });
        await idle();
        const before = await inventory();
        catalog[0].source += '#single-tool-check';
        catalog[0].id = identify(catalog[0]);
        await saveCatalog();
        await api('/admin/tools/yt-dlp/check', 'POST', {});
        const after = await idle();
        const item = after.find((value) => value.id === 'yt-dlp');
        assert.equal(item.candidate.id, catalog[0].id);
        assert.equal(
          item.installed.id,
          before.find((value) => value.id === 'yt-dlp').installed.id,
        );
        for (const other of after.filter((value) => value.id !== 'yt-dlp')) {
          assert.equal(
            other.checked_at,
            before.find((value) => value.id === other.id).checked_at,
            other.id,
          );
          assert.equal(
            other.job?.id,
            before.find((value) => value.id === other.id).job?.id,
            other.id,
          );
        }
        await api('/admin/tools/unknown/check', 'POST', {}, 404);
        await api(
          '/admin/tools/yt-dlp/install',
          'POST',
          { candidate_id: catalog[0].id },
          409,
        );
        await api('/admin/tools/yt-dlp/settings', 'POST', {
          policy: 'notify',
          channel: 'nightly',
          pinned: false,
        });
      },
    );
    await scenario(
      'a running conversion retains FFmpeg and a manual update waits until it ends',
      async () => {
        const ffmpeg = (await inventory()).find((item) => item.id === 'ffmpeg');
        const executable = `/var/lib/thelxinoe/tools/packages/${ffmpeg.installed.id}/ffmpeg`;
        await docker('exec', '--user', '0', name, 'mkdir', '-p', '/media');
        await docker(
          'exec',
          '--user',
          '0',
          name,
          'chown',
          '10001:10001',
          '/media',
        );
        await docker(
          'exec',
          name,
          executable,
          '-v',
          'error',
          '-f',
          'lavfi',
          '-i',
          'testsrc2=size=320x180:rate=24',
          '-f',
          'lavfi',
          '-i',
          'sine=frequency=440',
          '-t',
          '180',
          '-c:v',
          'libx264',
          '-preset',
          'ultrafast',
          '-threads',
          '1',
          '-c:a',
          'aac',
          '/media/Tools (2026).mp4',
        );
        await api('/catalog/roots', 'POST', {
          name: 'Tool fixtures',
          kind: 'movies',
          path: '/media',
        });
        const media = await eventually(async () =>
          (await api('/catalog')).items.find((item) => item.title === 'Tools'),
        );
        const playback = await api('/playback', 'POST', {
          media_id: media.id,
          options: {
            quality: '720p',
            capabilities: {
              containers: [],
              video: ['h264'],
              audio: ['aac'],
              hls: true,
            },
          },
        });
        const candidate = catalog.find((item) => item.tool === 'ffmpeg');
        candidate.source += '#qualification-build';
        candidate.id = identify(candidate);
        await saveCatalog();
        await check();
        await api('/admin/tools/ffmpeg/install', 'POST', {
          candidate_id: candidate.id,
        });
        const waiting = await eventually(async () => {
          await api(`/playback/${playback.id}/keepalive`, 'POST', {});
          const item = (await inventory()).find(
            (value) => value.id === 'ffmpeg',
          );
          assert.notEqual(item.job?.stage, 'failed', item.job?.error);
          return item.job?.stage === 'waiting' && item.job.reason && item;
        });
        assert.equal(waiting.installed.id, ffmpeg.installed.id);
        await docker('exec', name, executable, '-version');
        await api(`/playback/${playback.id}`, 'DELETE');
        const updated = (await idle()).find((value) => value.id === 'ffmpeg');
        assert.equal(updated.job.stage, 'complete', updated.job.error);
        assert.equal(updated.installed.candidate_id, candidate.id);
        return {
          waiting: waiting.job.reason,
          previous: updated.previous,
          installed: updated.installed,
        };
      },
    );
    await scenario(
      'Automatic prepares outside the window, a later Notify choice wins, and a manual click activates',
      async () => {
        const start = (new Date().getUTCHours() + 12) % 24;
        await api('/admin/product-update/policy', 'POST', {
          policy: 'notify',
          window_start: start,
          window_end: (start + 1) % 24,
        });
        const candidate = catalog.find((item) => item.tool === 'deno');
        candidate.source += '#qualification-build';
        candidate.id = identify(candidate);
        await saveCatalog();
        await check();
        await api('/admin/tools/deno/settings', 'POST', {
          policy: 'automatic',
          channel: 'lts',
          pinned: false,
        });
        await eventually(async () => {
          const item = (await inventory()).find((value) => value.id === 'deno');
          assert.notEqual(item.job?.stage, 'failed', item.job?.error);
          return (
            item.job?.stage === 'waiting' &&
            item.job.reason?.includes('maintenance')
          );
        });
        await api('/admin/tools/deno/settings', 'POST', {
          policy: 'notify',
          channel: 'lts',
          pinned: false,
        });
        await eventually(async () =>
          (await inventory())
            .find((item) => item.id === 'deno')
            .job.reason?.includes('policy'),
        );
        const checked = (await inventory()).find(
          (item) => item.id === 'deno',
        ).checked_at;
        await api('/admin/tools/check', 'POST', {});
        await eventually(
          async () =>
            (await inventory()).find((item) => item.id === 'deno').checked_at >
            checked,
        );
        await api('/admin/tools/deno/install', 'POST', {
          candidate_id: candidate.id,
        });
        const item = (await idle()).find((value) => value.id === 'deno');
        assert.equal(item.job.stage, 'complete', item.job.error);
        assert.equal(item.installed.candidate_id, candidate.id);
        return item;
      },
    );
    await scenario(
      'an altered candidate cannot replace the working package',
      async () => {
        const before = (await inventory()).find(
          (value) => value.id === 'yt-dlp',
        ).installed.id;
        catalog[0].artifacts[0].sha256 = '0'.repeat(64);
        await saveCatalog();
        await check();
        await api('/admin/tools/yt-dlp/install', 'POST', {
          candidate_id: catalog[0].id,
        });
        const item = (await idle()).find((value) => value.id === 'yt-dlp');
        assert.equal(item.job.stage, 'failed');
        assert.match(item.job.error, /identity changed/i);
        assert.equal(item.installed.id, before);
        return item;
      },
    );
    await scenario(
      'restart preserves selections and offline repair restores exact artifacts',
      async () => {
        const before = (await inventory()).map((item) => item.installed.id);
        await docker('restart', name);
        await eventually(() => api('/health'));
        assert.deepEqual(
          (await idle()).map((item) => item.installed.id),
          before,
        );
        const item = (await inventory()).find((value) => value.id === 'yt-dlp');
        await api('/admin/tools/yt-dlp/repair', 'POST', {
          candidate_id: item.installed.candidate_id,
        });
        const repaired = (await idle()).find((value) => value.id === 'yt-dlp');
        assert.equal(repaired.job.stage, 'complete', repaired.job.error);
        assert.equal(
          repaired.installed.candidate_id,
          item.installed.candidate_id,
        );
        assert.notEqual(repaired.installed.id, item.installed.id);
        return repaired;
      },
    );
    await scenario(
      'invalid platform, version, size and archive capabilities cannot replace FFmpeg',
      async () => {
        const index = catalog.findIndex((item) => item.tool === 'ffmpeg');
        const valid = structuredClone(catalog[index]);
        const before = (await inventory()).find((item) => item.id === 'ffmpeg')
          .installed.id;
        const failures = [];
        for (const failure of [
          'platform',
          'version',
          'size',
          'unsafe-archive',
          'missing-codecs',
        ]) {
          const candidate = structuredClone(valid);
          let fixture;
          if (failure === 'platform') candidate.platform = 'linux-aarch64';
          if (failure === 'version') candidate.version = '0.0';
          if (failure === 'size') candidate.artifacts[0].size = 3 * 1024 ** 3;
          if (['unsafe-archive', 'missing-codecs'].includes(failure)) {
            fixture = `import io,tarfile,hashlib,json\nfrom pathlib import Path\nb=io.BytesIO()\nwith tarfile.open(fileobj=b,mode='w:xz') as t:\n for name in ${failure === 'unsafe-archive' ? "['../escape']" : "['fixture/bin/ffmpeg','fixture/bin/ffprobe']"}:\n  data=b'#!/bin/sh\\necho ffmpeg version n8.1\\n'; i=tarfile.TarInfo(name); i.size=len(data); i.mode=0o755; t.addfile(i,io.BytesIO(data))\ndata=b.getvalue(); digest=hashlib.sha256(data).hexdigest(); Path('/var/lib/thelxinoe/tools/artifacts',digest).write_bytes(data); print(json.dumps({'sha256':digest,'size':len(data)}))`;
            Object.assign(
              candidate.artifacts[0],
              JSON.parse(await docker('exec', name, 'python3', '-c', fixture)),
            );
          }
          candidate.id = identify(candidate);
          catalog[index] = candidate;
          await saveCatalog();
          await check();
          if (fixture) await docker('exec', name, 'python3', '-c', fixture);
          await api('/admin/tools/ffmpeg/install', 'POST', {
            candidate_id: candidate.id,
          });
          const item = (await idle()).find((value) => value.id === 'ffmpeg');
          assert.equal(item.job.stage, 'failed', failure);
          const expected = {
            platform: /platform is unsupported/i,
            version: /build mismatch/i,
            size: /download limit/i,
            'unsafe-archive': /unsafe archive member/i,
            'missing-codecs': /missing required encoders/i,
          };
          assert.match(item.job.error, expected[failure]);
          assert.equal(item.installed.id, before, failure);
          failures.push({ failure, error: item.job.error });
        }
        catalog[index] = valid;
        await saveCatalog();
        await check();
        return failures;
      },
    );
    await scenario(
      'offline discovery preserves the catalog and persists a bounded retry',
      async () => {
        const before = await inventory();
        await docker(
          'exec',
          name,
          'python3',
          '-c',
          "from pathlib import Path;Path('/var/lib/thelxinoe/catalog.json').write_text('invalid')",
        );
        const items = await check();
        for (const [index, item] of items.entries()) {
          assert.equal(item.installed.id, before[index].installed.id);
          assert.deepEqual(item.candidate, before[index].candidate);
          assert.ok(item.check_error);
          assert.ok(
            item.next_check_at > Date.now() / 1000 &&
              item.next_check_at < Date.now() / 1000 + 86401,
          );
        }
        await saveCatalog();
        await check();
        return items.map(({ id, check_error, next_check_at }) => ({
          id,
          check_error,
          next_check_at,
        }));
      },
    );
    await scenario(
      'startup recovers a corrupt activated package and holds it',
      async () => {
        const before = (await inventory()).find((item) => item.id === 'ffmpeg');
        await docker('stop', name);
        await docker(
          'run',
          '--rm',
          '--network',
          'none',
          '-v',
          `${name}:/var/lib/thelxinoe`,
          '--entrypoint',
          'python3',
          image,
          '-c',
          `from pathlib import Path;Path('/var/lib/thelxinoe/tools/packages/${before.installed.id}/ffmpeg').write_bytes(b'corrupt')`,
        );
        await docker('start', name);
        await eventually(() => api('/health'));
        const item = (await idle()).find((value) => value.id === 'ffmpeg');
        assert.equal(item.installed.id, before.previous.id);
        assert.equal(item.held, before.installed.candidate_id);
        assert.equal(item.integrity_error, null);
        return item;
      },
    );
    await scenario(
      'a prepared YouTube runtime pair activates and rolls back as one group',
      async () => {
        const before = (await inventory()).filter((item) =>
          ['yt-dlp', 'deno'].includes(item.id),
        );
        const seeds = JSON.parse(
          readFileSync('scripts/server-tools.lock.json', 'utf8'),
        );
        for (const id of ['yt-dlp', 'deno']) {
          const candidate = seeds.find((item) => item.tool === id);
          candidate.source += '#paired-qualification';
          candidate.id = identify(candidate);
          catalog[catalog.findIndex((item) => item.tool === id)] = candidate;
        }
        await saveCatalog();
        await check();
        await Promise.all(
          ['yt-dlp', 'deno'].map((id) =>
            api(`/admin/tools/${id}/install`, 'POST', {
              candidate_id: catalog.find((item) => item.tool === id).id,
            }),
          ),
        );
        const activated = (await idle()).filter((item) =>
          ['yt-dlp', 'deno'].includes(item.id),
        );
        for (const item of activated)
          assert.equal(item.job.stage, 'complete', item.job.error);
        await api('/admin/tools/yt-dlp/rollback', 'POST', {
          candidate_id: before[0].installed.candidate_id,
        });
        const rolledBack = (await idle()).filter((item) =>
          ['yt-dlp', 'deno'].includes(item.id),
        );
        assert.deepEqual(
          rolledBack.map((item) => item.installed.id),
          before.map((item) => item.installed.id),
        );
        return rolledBack;
      },
    );
    await scenario(
      'interrupted publication is reconciled and interpreter changes rebuild exact wheels offline',
      async () => {
        const before = (await inventory()).find(
          (item) => item.id === 'streamlink',
        ).installed;
        await docker('stop', name);
        await docker(
          'run',
          '--rm',
          '--network',
          'none',
          '-v',
          `${name}:/var/lib/thelxinoe`,
          '--entrypoint',
          'python3',
          image,
          '-c',
          `import json,sqlite3\nfrom pathlib import Path\nroot=Path('/var/lib/thelxinoe'); db=sqlite3.connect(root/'thelxinoe.sqlite3'); row=db.execute("SELECT id,manifest FROM tool_generations WHERE id=?",('${before.id}',)).fetchone(); manifest=json.loads(row[1]); manifest['python_abi']='fixture-old-interpreter'; db.execute('UPDATE tool_generations SET manifest=? WHERE id=?',(json.dumps(manifest),row[0])); db.commit(); db.close(); (root/'tools/packages/.stage-interrupted').mkdir(); (root/'tools/packages'/('f'*32)).mkdir()`,
        );
        await docker(
          'run',
          '--rm',
          '--network',
          'none',
          '-v',
          `${name}:/var/lib/thelxinoe`,
          image,
          'validate-state',
        );
        await docker('start', name);
        await eventually(() => api('/health'));
        const item = (await idle()).find((value) => value.id === 'streamlink');
        assert.notEqual(item.installed.id, before.id);
        assert.equal(item.installed.candidate_id, before.candidate_id);
        assert.equal(item.integrity_error, null);
        const leftovers = JSON.parse(
          await docker(
            'exec',
            name,
            'python3',
            '-c',
            "import json;from pathlib import Path;p=Path('/var/lib/thelxinoe/tools/packages');print(json.dumps([(p/'.stage-interrupted').exists(),(p/('f'*32)).exists()]))",
          ),
        );
        assert.deepEqual(leftovers, [false, false]);
        return item;
      },
    );
    await scenario(
      'an offline state snapshot restores selected packages and recovery locks',
      async () => {
        const before = (await inventory()).map(
          ({ id, installed, previous, held }) => ({
            id,
            installed,
            previous,
            held,
          }),
        );
        await docker('stop', name);
        await docker('volume', 'create', `${name}-restored`);
        await docker(
          'run',
          '--rm',
          '--user',
          '0',
          '--network',
          'none',
          '-v',
          `${name}:/source:ro`,
          '-v',
          `${name}-restored:/restore`,
          '--entrypoint',
          'python3',
          image,
          '-c',
          "import shutil,os;from pathlib import Path;shutil.copytree('/source','/restore',dirs_exist_ok=True);[os.chown(p,10001,10001) for p in [Path('/restore'),*Path('/restore').rglob('*')]]",
        );
        await docker(
          'run',
          '--rm',
          '--network',
          'none',
          '-v',
          `${name}-restored:/var/lib/thelxinoe`,
          image,
          'validate-state',
        );
        await docker('rm', name);
        await docker(
          'run',
          '-d',
          '--name',
          name,
          '--network',
          name,
          '-v',
          `${name}-restored:/var/lib/thelxinoe`,
          '-e',
          'THELXINOE_TOOLS_CATALOG=/var/lib/thelxinoe/catalog.json',
          image,
        );
        await eventually(() => api('/health'));
        assert.deepEqual(
          (await idle()).map(({ id, installed, previous, held }) => ({
            id,
            installed,
            previous,
            held,
          })),
          before,
        );
        return before;
      },
    );
    await scenario(
      'an interrupted transfer retains its frozen candidate and a channel change cancels it',
      async () => {
        const index = catalog.findIndex((item) => item.tool === 'yt-dlp');
        const previous = structuredClone(catalog[index]);
        const before = (await inventory()).find(
          (item) => item.id === 'yt-dlp',
        ).installed;
        catalog[index].artifacts[0].sha256 = 'e'.repeat(64);
        catalog[index].id = identify(catalog[index]);
        await saveCatalog();
        await check();
        await api('/admin/tools/yt-dlp/install', 'POST', {
          candidate_id: catalog[index].id,
        });
        const waiting = await eventually(async () => {
          const item = (await inventory()).find((item) => item.id === 'yt-dlp');
          assert.notEqual(item.job.stage, 'failed', item.job.error);
          return item.job.retry_at > Date.now() / 1000 && item;
        });
        assert.equal(waiting.installed.id, before.id);
        assert.equal(waiting.job.candidate.id, catalog[index].id);
        await api('/admin/tools/yt-dlp/settings', 'POST', {
          policy: 'notify',
          channel: 'stable',
          pinned: false,
        });
        await eventually(
          async () =>
            (await inventory()).find((item) => item.id === 'yt-dlp').job
              .stage === 'canceled',
        );
        catalog[index] = previous;
        await saveCatalog();
        await api('/admin/tools/yt-dlp/settings', 'POST', {
          policy: 'notify',
          channel: 'nightly',
          pinned: false,
        });
        await check();
        return waiting.job;
      },
    );
    evidence.product_passed = true;
  } catch (error) {
    evidence.error = error.stack ?? String(error);
    throw error;
  } finally {
    try {
      writeFileSync(resolve(output, 'server.log'), await docker('logs', name));
    } catch {
      /* Container may not have started. */
    }
    try {
      resources.close();
    } catch (error) {
      evidence.cleanup_errors.push(error.message);
    }
    evidence.finished = new Date().toISOString();
    evidence.passed =
      evidence.product_passed && !evidence.cleanup_errors.length;
    save();
  }
}
await qualify();
if (!evidence.passed) process.exitCode = 1;
