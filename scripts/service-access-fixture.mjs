import { expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { resolve } from 'node:path';
import {
  composeFixture,
  fixtureId,
  freePort,
  resourceScope,
} from './ci-resources.mjs';
import { fixtureImage } from './ci-images.mjs';
import {
  budgets,
  requestBudget,
  waitForState,
  waitForProvision,
} from './ci-readiness.mjs';
import { launchBrowser } from './ci-browser.mjs';
export { freePort } from './ci-resources.mjs';

export const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();

export async function waitForProxy(client, base) {
  await waitForState(
    'Authenticated proxy startup',
    async (timeout) => {
      const response = await client.get(`${base}/api/v1/health`, { timeout });
      return {
        http_status: response.status(),
        status: (await response.json()).status,
      };
    },
    (value) => value.http_status === 200 && value.status === 'ok',
    { timeout: budgets.startup },
  );
}

export async function fixture({ scheme = 'https' } = {}) {
  resourceScope();
  const project = fixtureId('access');
  const root = resolve(`.local/${project}`);
  const port = await freePort();
  const base = `${scheme}://localhost:${port}`;
  const env = {
    ...process.env,
    THELXINOE_CONNECTIONS_ROOT: root,
    THELXINOE_CONNECTIONS_PORT: String(port),
    THELXINOE_CONNECTIONS_URL: base,
    THELXINOE_CONNECTIONS_PROXY_PORT: scheme === 'https' ? '9443' : '9080',
  };
  for (const directory of [
    'server',
    'cache',
    'media/movies',
    'media/tv',
    'media/music',
    'media/downloads',
  ])
    mkdirSync(`${root}/${directory}`, { recursive: true });
  let infrastructure;
  const compose = (...args) => infrastructure.compose(...args);
  let browser, context;
  const services = {};
  const attachedContainers = [];
  const secrets = new Set(['test-only long passphrase']);
  let closed = false;
  let deployment;
  async function api(
    path,
    method = 'GET',
    data,
    client = context.request,
    timeout = requestBudget(path),
  ) {
    const response = await client.fetch(`${base}/api/v1${path}`, {
      method,
      data,
      timeout,
      headers: { 'X-Thelxinoe-Client': '1' },
    });
    if (!response.ok()) {
      const body = await response.json().catch(() => ({}));
      throw Error(
        `${path}: HTTP ${response.status()} ${body.error?.message ?? ''}`,
      );
    }
    return response.json();
  }
  const stack = (timeout) =>
    api('/admin/stack', 'GET', undefined, context.request, timeout);
  async function install(kind) {
    const host_port = await freePort();
    const created = await api('/admin/stack/install', 'POST', {
      kind,
      host_port,
    });
    let latest;
    try {
      latest = (
        await waitForProvision(
          async (timeout) => {
            latest = await stack(timeout);
            return latest;
          },
          created.id,
          kind,
        )
      ).stack;
    } catch (error) {
      const containers = [];
      for (const item of latest?.items ?? []) {
        if (!item.container_id) continue;
        try {
          const [raw] = JSON.parse(docker('inspect', item.container_id));
          const {
            Status,
            Running,
            Restarting,
            OOMKilled,
            ExitCode,
            Error: failure,
            StartedAt,
            FinishedAt,
          } = raw.State;
          containers.push({
            kind: item.kind,
            id: raw.Id,
            image: raw.Image,
            Status,
            Running,
            Restarting,
            OOMKilled,
            ExitCode,
            failure,
            StartedAt,
            FinishedAt,
          });
        } catch (inspection) {
          containers.push({ kind: item.kind, error: inspection.message });
        }
      }
      mkdirSync('test-results/service-access', { recursive: true });
      writeFileSync(
        `test-results/service-access/${project}-${kind}-failure.json`,
        JSON.stringify(
          { project, kind, error: error.message, stack: latest, containers },
          null,
          2,
        ),
      );
      throw error;
    }
    services[kind] = {
      ...latest.items.find((s) => s.id === created.id),
      host_port,
    };
    return services[kind];
  }
  function config(kind) {
    const filename =
      {
        bazarr: 'config/config.yaml',
        nzbget: 'nzbget.conf',
        seerr: 'settings.json',
      }[kind] ?? 'config.xml';
    const raw = compose(
      'exec',
      '-T',
      '-u',
      '10001:10001',
      'controller',
      'cat',
      `/var/lib/thelxinoe/deployment/services/${services[kind].id}/appdata/${filename}`,
    );
    for (const pattern of [
      /<ApiKey>(.*?)<\/ApiKey>/,
      /^\s+apikey:\s*['"]?([a-zA-Z0-9]+)/m,
      /^ControlPassword=(.*)$/m,
      /"apiKey"\s*:\s*"([^"]+)"/,
    ]) {
      const secret = raw.match(pattern)?.[1]?.trim();
      if (secret) secrets.add(secret);
    }
    return raw;
  }
  async function upstream(kind, path, method = 'GET', data) {
    const raw = config(kind);
    if (kind === 'nzbget') {
      const username = raw.match(/^ControlUsername=(.*)$/m)[1].trim();
      const password = raw.match(/^ControlPassword=(.*)$/m)[1].trim();
      const response = await context.request.post(
        `http://localhost:${services[kind].host_port}/jsonrpc`,
        {
          headers: {
            Authorization: `Basic ${Buffer.from(`${username}:${password}`).toString('base64')}`,
          },
          data: { method: path, params: data ?? [], id: 1 },
        },
      );
      if (!response.ok())
        throw Error(`NZBGet/${path}: HTTP ${response.status()}`);
      const body = await response.json();
      if (body.error) throw Error(`NZBGet/${path}: RPC rejected`);
      return body.result;
    }
    const key =
      kind === 'bazarr'
        ? raw.match(/^\s+apikey:\s*['"]?([a-zA-Z0-9]+)/m)[1]
        : kind === 'seerr'
          ? JSON.parse(raw).main.apiKey
          : raw.match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
    const prefix =
      kind === 'bazarr'
        ? (
            raw.match(
              /^general:\r?\n(?:[ \t].*\r?\n)*?[ \t]+base_url:\s*([^\r\n]+)/m,
            )?.[1] ?? ''
          )
            .trim()
            .replace(/^['"]|['"]$/g, '')
            .replace(/\/$/, '')
        : (raw.match(/<UrlBase>(.*?)<\/UrlBase>/)?.[1] ?? '');
    const version = ['lidarr', 'prowlarr', 'seerr'].includes(kind) ? 1 : 3;
    const response = await context.request.fetch(
      `http://localhost:${services[kind].host_port}${prefix}/api/${kind === 'bazarr' ? '' : `v${version}/`}${path}`,
      { method, data, headers: { 'X-Api-Key': key } },
    );
    if (!response.ok())
      throw Error(`${kind}/${path}: HTTP ${response.status()}`);
    return response.status() === 204 ||
      response.headers()['content-length'] === '0'
      ? null
      : response.json();
  }
  async function close() {
    if (closed) return;
    if (infrastructure) {
      try {
        for (const kind of Object.keys(services)) {
          try {
            config(kind);
          } catch {
            /* The failed service may already be unavailable. */
          }
        }
        let logs = compose('logs', '--no-color', 'server', 'controller');
        for (const secret of secrets)
          logs = logs.replaceAll(secret, '[redacted]');
        writeFileSync(`${root}/services.log`, logs);
      } catch (error) {
        console.error('Could not capture fixture diagnostics:', error.message);
      }
    }
    try {
      await browser?.close();
    } finally {
      infrastructure?.close();
    }
    closed = true;
  }

  async function attach(kind, urlBase, image) {
    const name = `${project}-attached-${kind}-${attachedContainers.length}`;
    const directory = `${root}/attached-${kind}-${attachedContainers.length}`;
    const internalPort = {
      radarr: 7878,
      sonarr: 8989,
      lidarr: 8686,
      prowlarr: 9696,
      bazarr: 6767,
      nzbget: 6789,
    }[kind];
    const port = await freePort();
    const key = randomBytes(16).toString('hex');
    secrets.add(key);
    mkdirSync(directory, { recursive: true });
    if (kind === 'bazarr') {
      mkdirSync(`${directory}/config`, { recursive: true });
      writeFileSync(
        `${directory}/config/config.yaml`,
        `auth:\n  apikey: ${key}\ngeneral:\n  hostname: attached-bazarr\n  ip: 0.0.0.0\n  port: 6767\n  base_url: ${urlBase || '/'}\n  use_sonarr: false\n  use_radarr: false\n`,
      );
    } else if (kind === 'nzbget') {
      writeFileSync(
        `${directory}/nzbget.conf`,
        `MainDir=/media/downloads\nDestDir=/media/downloads/completed\nInterDir=/media/downloads/intermediate\nNzbDir=/config/nzb\nQueueDir=/config/queue\nTempDir=/config/tmp\nWebDir=\${AppDir}/webui\nConfigTemplate=\${AppDir}/webui/nzbget.conf.template\nControlIP=0.0.0.0\nControlPort=6789\nControlUsername=fixture\nControlPassword=${key}\n`,
      );
    } else
      writeFileSync(
        `${directory}/config.xml`,
        `<Config><BindAddress>*</BindAddress><Port>${internalPort}</Port><UrlBase>${urlBase}</UrlBase>${kind === 'prowlarr' ? `<AllowedHosts>${name}</AllowedHosts>` : ''}<EnableSsl>False</EnableSsl><LaunchBrowser>False</LaunchBrowser><ApiKey>${key}</ApiKey><AuthenticationMethod>External</AuthenticationMethod><AuthenticationRequired>Enabled</AuthenticationRequired><UpdateAutomatically>False</UpdateAutomatically></Config>`,
      );
    attachedContainers.push(name);
    infrastructure.update({ containers: attachedContainers });
    const container = docker(
      'run',
      '-d',
      '--label',
      `io.thelxinoe.ci-run=${env.THELXINOE_CI_RUN_ID ?? project}`,
      '--name',
      name,
      '--network',
      `${project}_test`,
      '-e',
      'PUID=10001',
      '-e',
      'PGID=10001',
      '-v',
      `${directory}:/config`,
      ...(kind === 'prowlarr' ? [] : ['-v', `${mediaSource()}:/media`]),
      '-p',
      `127.0.0.1:${port}:${internalPort}`,
      image,
    );
    const direct = async (path, method = 'GET', data) => {
      if (kind === 'nzbget') {
        const response = await context.request.post(
          `http://localhost:${port}/jsonrpc`,
          {
            headers: {
              Authorization: `Basic ${Buffer.from(`fixture:${key}`).toString('base64')}`,
            },
            data: { method: path, params: data ?? [], id: 1 },
          },
        );
        if (!response.ok())
          throw Error(`Attached NZBGet/${path}: HTTP ${response.status()}`);
        const body = await response.json();
        if (body.error) throw Error(`Attached NZBGet/${path}: RPC rejected`);
        return body.result;
      }
      const response = await context.request.fetch(
        `http://localhost:${port}${urlBase}/api/${kind === 'bazarr' ? '' : `v${['lidarr', 'prowlarr'].includes(kind) ? 1 : 3}/`}${path}`,
        {
          method,
          data,
          headers: {
            'X-Api-Key': key,
            ...(kind === 'prowlarr' ? { Host: `${name}:${internalPort}` } : {}),
          },
        },
      );
      if (!response.ok())
        throw Error(`Attached ${kind}/${path}: HTTP ${response.status()}`);
      const text = await response.text();
      return text ? JSON.parse(text) : null;
    };
    async function register() {
      await waitForState(
        `Attached ${kind} startup`,
        async () => {
          if (kind === 'nzbget') return (await direct('version')) ? kind : '';
          const status = await direct('system/status');
          return kind === 'bazarr' && status.data.bazarr_version
            ? kind
            : status.appName.toLowerCase();
        },
        (value) => value === kind,
        { timeout: budgets.startup },
      );
      const support = ['prowlarr', 'bazarr', 'nzbget'].includes(kind);
      return api(support ? '/admin/support' : '/admin/managers', 'POST', {
        name: `Attached ${kind}`,
        kind,
        container_id: container,
        port: internalPort,
        ...(support
          ? {
              credentials: {
                username: kind === 'nzbget' ? 'fixture' : '',
                secret: key,
              },
            }
          : { api_key: key }),
        url_base: urlBase,
      });
    }
    const registered = await register();
    return {
      id: registered.id,
      container,
      direct,
      port,
      directory,
      // Simulate an external owner's config change and subsequent reconnection.
      async setBase(base) {
        if (!['radarr', 'sonarr', 'lidarr', 'prowlarr'].includes(kind))
          throw Error('This fixture changes XML service configuration only');
        docker('stop', container);
        const path = `${directory}/config.xml`;
        // The service owns this file after startup, including on Linux hosts.
        execFileSync(
          'docker',
          [
            'run',
            '--label',
            `com.docker.compose.project=${project}`,
            '--label',
            `io.thelxinoe.ci-run=${process.env.THELXINOE_CI_RUN_ID}`,
            '--rm',
            '-i',
            '--network',
            'none',
            '--user',
            '10001:10001',
            '--volumes-from',
            container,
            '--entrypoint',
            'sh',
            image,
            '-c',
            'cat > /config/config.xml',
          ],
          {
            input: readFileSync(path, 'utf8').replace(
              /<UrlBase>.*?<\/UrlBase>/s,
              `<UrlBase>${base}</UrlBase>`,
            ),
            stdio: ['pipe', 'pipe', 'pipe'],
          },
        );
        urlBase = base;
        docker('start', container);
        await register();
      },
    };
  }
  async function peer() {
    const name = `${project}-peer`;
    attachedContainers.push(name);
    infrastructure.update({ containers: attachedContainers });
    const container = docker(
      'run',
      '-d',
      '--label',
      `io.thelxinoe.ci-run=${env.THELXINOE_CI_RUN_ID ?? project}`,
      '--name',
      name,
      '--network',
      `${project}_test`,
      '-v',
      `${mediaSource()}:/media`,
      '-v',
      `${resolve('tests/service-access-peer.mjs')}:/peer.mjs:ro`,
      fixtureImage('node:24-bookworm-slim'),
      'node',
      '/peer.mjs',
    );
    let registered;
    await expect
      .poll(
        async () => {
          try {
            registered = await api('/admin/managers', 'POST', {
              name: 'Gateway boundary fixture',
              kind: 'radarr',
              container_id: container,
              port: 7878,
              api_key: 'fixture-key-not-a-secret',
              url_base: '/services/radarr',
            });
            return true;
          } catch {
            return false;
          }
        },
        { timeout: 30000 },
      )
      .toBe(true);
    return registered;
  }
  function mediaSource() {
    // Docker Desktop may spell the same Windows bind differently for Compose
    // and `docker run`. Use the daemon's exact source, as real attachments must.
    const mounts = JSON.parse(
      docker('inspect', '--format', '{{json .Mounts}}', `${project}-server-1`),
    );
    const source = mounts.find(
      (mount) => mount.Destination === '/media',
    )?.Source;
    if (!source) throw Error('Fixture media mount is missing');
    return source;
  }
  try {
    browser = await launchBrowser();
    context = await browser.newContext({ ignoreHTTPSErrors: true });
    infrastructure = composeFixture({
      project,
      file: 'compose.connections.test.yaml',
      root,
      env,
    });
    compose(
      'run',
      '--rm',
      '--no-deps',
      '--user',
      '0:0',
      '--entrypoint',
      'chown',
      'server',
      '-R',
      '10001:10001',
      '/var/lib/thelxinoe',
      '/var/cache/thelxinoe',
      '/media',
    );
    compose('up', '-d', '--wait', '--wait-timeout', '180');
    await waitForProxy(context.request, base);
    await api('/setup', 'POST', {
      username: 'admin',
      password: 'test-only long passphrase',
    });
    deployment = (await stack()).deployment_id;
    infrastructure.update({ deployment });
    return {
      project,
      root,
      base,
      browser,
      context,
      api,
      install,
      services,
      config,
      upstream,
      stack,
      compose,
      close,
      attach,
      peer,
    };
  } catch (error) {
    const output = 'test-results/service-access';
    mkdirSync(output, { recursive: true });
    try {
      writeFileSync(
        `${output}/${project}-startup.log`,
        compose('logs', '--no-color', 'controller', 'server', 'proxy'),
      );
    } catch (diagnosticError) {
      console.error(
        'Could not collect fixture startup logs:',
        diagnosticError.message,
      );
    }
    await close();
    throw error;
  }
}
