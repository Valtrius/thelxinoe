import { chromium, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { resolve } from 'node:path';
import { createServer } from 'node:net';

export const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();

export async function freePort() {
  const listener = createServer();
  await new Promise((done, reject) => {
    listener.once('error', reject);
    listener.listen(0, '127.0.0.1', done);
  });
  const port = listener.address().port;
  await new Promise((done) => listener.close(done));
  return port;
}

export async function waitForProxy(client, base) {
  await expect
    .poll(
      async () => {
        try {
          const response = await client.get(`${base}/api/v1/health`, {
            timeout: 5000,
          });
          return response.ok() && (await response.json()).status === 'ok';
        } catch {
          return false;
        }
      },
      { timeout: 30000, intervals: [500, 1000] },
    )
    .toBe(true);
}

export async function fixture({ scheme = 'https' } = {}) {
  const project = `thelxinoe-access-${Date.now()}`;
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
  const compose = (...args) =>
    execFileSync(
      'docker',
      [
        'compose',
        '-p',
        project,
        '-f',
        'compose.connections.test.yaml',
        ...args,
      ],
      { env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] },
    ).trim();
  const browser = await chromium.launch();
  const context = await browser.newContext({ ignoreHTTPSErrors: true });
  const services = {};
  const attachedContainers = [];
  let closed = false;
  let deployment;
  async function api(path, method = 'GET', data, client = context.request) {
    const response = await client.fetch(`${base}/api/v1${path}`, {
      method,
      data,
      headers: { 'X-Thelxinoe-Client': '1' },
    });
    if (!response.ok()) throw Error(`${path}: HTTP ${response.status()}`);
    return response.json();
  }
  const stack = () => api('/admin/stack');
  async function install(kind) {
    const host_port = await freePort();
    const created = await api('/admin/stack/install', 'POST', {
      kind,
      host_port,
    });
    let latest;
    await expect
      .poll(
        async () => {
          latest = await stack();
          const provision = latest.provisions.find((p) => p.id === created.id);
          if (provision?.state === 'blocked')
            throw Error(`${kind}: ${provision.error}`);
          return provision?.state;
        },
        { timeout: 300000, intervals: [2000] },
      )
      .toBe('complete');
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
    return compose(
      'exec',
      '-T',
      '-u',
      '10001:10001',
      'controller',
      'cat',
      `/var/lib/thelxinoe/deployment/services/${services[kind].id}/appdata/${filename}`,
    );
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
    closed = true;
    await browser.close();
    if (attachedContainers.length) docker('rm', '-f', ...attachedContainers);
    if (deployment) {
      const containers = docker(
        'ps',
        '-aq',
        '--filter',
        `label=app.thelxinoe.deployment=${deployment}`,
      )
        .split(/\s+/)
        .filter(Boolean);
      if (containers.length) docker('rm', '-f', ...containers);
    }
    compose('down', '-v');
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
    const container = docker(
      'run',
      '-d',
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
    attachedContainers.push(container);
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
      await expect
        .poll(
          async () => {
            try {
              if (kind === 'nzbget')
                return (await direct('version')) ? kind : '';
              const status = await direct('system/status');
              return kind === 'bazarr' && status.data.bazarr_version
                ? kind
                : status.appName.toLowerCase();
            } catch {
              return '';
            }
          },
          { timeout: 90000, intervals: [1000] },
        )
        .toBe(kind);
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
    const container = docker(
      'run',
      '-d',
      '--name',
      `${project}-peer`,
      '--network',
      `${project}_test`,
      '-v',
      `${mediaSource()}:/media`,
      '-v',
      `${resolve('tests/service-access-peer.mjs')}:/peer.mjs:ro`,
      'node:24-bookworm-slim',
      'node',
      '/peer.mjs',
    );
    attachedContainers.push(container);
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
