// Assemble publication from final Windows bytes and immutable Linux image identities.
import {
  readFileSync,
  writeFileSync,
  mkdirSync,
  mkdtempSync,
  copyFileSync,
  readdirSync,
  rmSync,
  unlinkSync,
} from 'node:fs';
import { execFileSync } from 'node:child_process';
import { resolve, basename, join } from 'node:path';
import { tmpdir } from 'node:os';
import { createPublicKey, createHash } from 'node:crypto';
import {
  checkSums,
  checkValidity,
  completeDraftAsset,
  draftGuard,
  feed,
  requireNewer,
  verifyEnvelope,
} from './release-checks.mjs';

const [command, ...args] = process.argv.slice(2);
const version = JSON.parse(readFileSync('package.json', 'utf8')).version;
const tag = version;
// Deployments follow the moving `latest` tags, so `docker compose pull` always
// reaches the newest release whatever version a host currently runs.
const releaseCompose = () => readFileSync('compose.release.yaml', 'utf8');
const run = (exe, args, options = {}) =>
  execFileSync(exe, args, { stdio: 'inherit', ...options });
const gh = (...args) =>
  execFileSync('gh', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
const sha256 = (file) =>
  createHash('sha256').update(readFileSync(file)).digest('hex');
// Images carry the commit they were built from; releases ship the tagged commit.
const revision = () =>
  execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
/** The latest published release tag, or null before the first release. */
function latestRelease() {
  try {
    return gh(
      'api',
      'repos/Valtrius/thelxinoe/releases/latest',
      '--jq',
      '.tag_name',
    );
  } catch (error) {
    if (/HTTP 404/.test(String(error.stderr))) return null;
    throw error;
  }
}
/**
 * The digest an anonymous Docker host resolves for `repository:reference`, or
 * null when it is absent or not public. Other registry errors are retried and
 * then thrown, so a GHCR hiccup never reads as a missing tag.
 */
async function registryDigest(repository, reference) {
  const path = repository.replace(/^ghcr\.io\//, '');
  const absent = (response) => [401, 403, 404].includes(response.status);
  for (let attempt = 1; ; attempt++) {
    try {
      const token = await fetch(
        `https://ghcr.io/token?scope=repository:${path}:pull`,
        { signal: AbortSignal.timeout(30000) },
      );
      if (absent(token)) return null;
      if (!token.ok) throw Error(`GHCR token HTTP ${token.status}`);
      const response = await fetch(
        `https://ghcr.io/v2/${path}/manifests/${reference}`,
        {
          method: 'HEAD',
          signal: AbortSignal.timeout(30000),
          headers: {
            Authorization: `Bearer ${(await token.json()).token}`,
            Accept: [
              'application/vnd.oci.image.manifest.v1+json',
              'application/vnd.docker.distribution.manifest.v2+json',
              'application/vnd.oci.image.index.v1+json',
              'application/vnd.docker.distribution.manifest.list.v2+json',
            ].join(', '),
          },
        },
      );
      if (absent(response)) return null;
      const digest = response.headers.get('docker-content-digest');
      if (!response.ok || !digest)
        throw Error(`GHCR ${path}:${reference} HTTP ${response.status}`);
      return digest;
    } catch (error) {
      if (attempt === 4) throw error;
      console.log(`Retrying after: ${error.message}`);
      await new Promise((done) => setTimeout(done, 5000 * attempt));
    }
  }
}
const verify = (directory, installer) =>
  run('cargo', [
    'run',
    '--locked',
    '-p',
    'thelxinoe-releases',
    '--bin',
    'verify-release',
    '--',
    resolve(directory, 'latest.json'),
    'releases/release.pub',
    installer,
  ]);
if (command === 'check') {
  const [requested] = args;
  if (requested !== version || !/^\d+\.\d+\.\d+$/.test(requested))
    throw Error(`Release tag must be ${version}`);
  execFileSync(process.execPath, ['scripts/version.mjs'], { stdio: 'inherit' });
} else if (command === 'desktop') {
  if (process.platform !== 'win32')
    throw Error('Release desktop builds require Windows');
  for (const key of ['TAURI_SIGNING_PRIVATE_KEY', 'TAURI_UPDATER_PUBLIC_KEY'])
    if (!process.env[key]) throw Error(`Missing ${key}`);
  for (const key of [
    'THELXINOE_RELEASE_URL',
    'THELXINOE_RELEASE_PUBLIC_KEY',
    'THELXINOE_RELEASE_CA_PEM',
  ])
    if (process.env[key])
      throw Error(`Remove test override ${key} from release builds`);
  mkdirSync('.local', { recursive: true });
  const config = resolve('.local/desktop-release.json');
  const settings = JSON.parse(
    readFileSync('apps/desktop/tauri.release.conf.json', 'utf8'),
  );
  settings.plugins.updater.pubkey = process.env.TAURI_UPDATER_PUBLIC_KEY.trim();
  writeFileSync(config, JSON.stringify(settings));
  const cli = resolve('node_modules/@tauri-apps/cli/tauri.js');
  try {
    run(process.execPath, [
      cli,
      'icon',
      'frontend/public/icon.svg',
      '--output',
      'apps/desktop/icons',
    ]);
    run(process.execPath, [cli, 'build', '--config', config], {
      cwd: 'apps/desktop',
    });
    mkdirSync('release-desktop', { recursive: true });
    const folder = 'target/release/bundle/nsis';
    const file = readdirSync(folder).find((name) =>
      name.endsWith(`_${version}_x64-setup.exe`),
    );
    if (!file) throw Error('Missing release installer');
    for (const name of [file, `${file}.sig`])
      copyFileSync(resolve(folder, name), resolve('release-desktop', name));
    writeFileSync(
      'release-desktop/updater.pub',
      settings.plugins.updater.pubkey + '\n',
    );
  } finally {
    unlinkSync(config);
  }
} else if (command === 'assemble') {
  const [
    directory,
    installer,
    signature,
    updaterKey,
    privateKey,
    serverRef,
    controllerRef,
  ] = args;
  if (!controllerRef)
    throw Error(
      'assemble: output installer signature updater.pub signing.pem server@digest controller@digest',
    );
  const output = resolve(directory);
  mkdirSync(output, { recursive: true });
  const expected = readFileSync('releases/release.pub', 'utf8').trim();
  const actual = createPublicKey(readFileSync(privateKey))
    .export({ type: 'spki', format: 'der' })
    .subarray(-32)
    .toString('base64');
  if (expected !== actual)
    throw Error('Publisher private key does not match releases/release.pub');
  const commit = revision();
  function image(reference) {
    if (!/@sha256:[a-f0-9]{64}$/.test(reference))
      throw Error('Release images must use registry digests');
    execFileSync('docker', ['pull', '--platform', 'linux/amd64', reference], {
      stdio: 'inherit',
    });
    const [image] = JSON.parse(
      execFileSync('docker', ['image', 'inspect', reference], {
        encoding: 'utf8',
      }),
    );
    if (image.Os !== 'linux' || image.Architecture !== 'amd64')
      throw Error('Unsupported image platform');
    if (image.Config.Labels['org.opencontainers.image.version'] !== version)
      throw Error('Image version does not match release');
    if (image.Config.Labels['org.opencontainers.image.revision'] !== commit)
      throw Error('Image revision does not match the release commit');
    if (!image.RepoDigests.includes(reference))
      throw Error('Pulled image does not match the registry digest');
    return { reference };
  }
  const database = readFileSync('crates/database/src/lib.rs', 'utf8');
  const schema = Number(
    database.match(/pub const SCHEMA_VERSION: u32 = (\d+)/)[1],
  );
  // Servers on any older schema this release still migrates may update to it.
  const base = Number(
    database.match(/pub const MIGRATION_BASE: u32 = (\d+)/)[1],
  );
  const core = readFileSync('crates/core/src/lib.rs', 'utf8');
  // Client APIs this release's server accepts; its desktop speaks the maximum.
  const api = {
    min: Number(core.match(/pub const API_MIN_VERSION: u32 = (\d+)/)[1]),
    max: Number(core.match(/pub const API_VERSION: u32 = (\d+)/)[1]),
  };
  const published = Math.floor(Date.now() / 1000);
  const file = basename(installer);
  const manifest = {
    format: 1,
    version,
    published_at: published,
    expires_at: published + 365 * 86400,
    server: image(serverRef),
    controller: image(controllerRef),
    windows_x64: {
      url: `https://github.com/Valtrius/thelxinoe/releases/download/${tag}/${file}`,
    },
    api,
    migration: {
      from: { min: base, max: schema },
      target: schema,
      recovery: 'full-state-restore',
      recovery_protocol: 1,
    },
    notes:
      process.env.RELEASE_NOTES ||
      `Thelxinoe ${version}. Release notes: https://github.com/Valtrius/thelxinoe/releases/tag/${tag}`,
  };
  const draft = resolve(output, 'draft.json');
  writeFileSync(draft, JSON.stringify(manifest, null, 2));
  execFileSync(
    process.execPath,
    [
      'scripts/assemble-release.mjs',
      draft,
      installer,
      signature,
      updaterKey,
      resolve(output, 'manifest.json'),
    ],
    { stdio: 'inherit' },
  );
  execFileSync(
    process.execPath,
    [
      'scripts/sign-release.mjs',
      resolve(output, 'manifest.json'),
      privateKey,
      resolve(output, 'latest.json'),
    ],
    { stdio: 'inherit' },
  );
  copyFileSync(installer, resolve(output, file));
  copyFileSync(signature, resolve(output, `${file}.sig`));
  copyFileSync('releases/release.pub', resolve(output, 'release.pub'));
  writeFileSync(resolve(output, 'compose.yaml'), releaseCompose());
  copyFileSync('.env.example', resolve(output, 'env.example'));
  verify(output, resolve(output, file));
  const assets = [
    file,
    `${file}.sig`,
    'latest.json',
    'windows-x64.json',
    'release.pub',
    'compose.yaml',
    'env.example',
  ];
  writeFileSync(
    resolve(output, 'SHA256SUMS'),
    assets
      .map((name) => `${sha256(resolve(output, name))}  ${name}`)
      .join('\n') + '\n',
  );
} else if (command === 'draft') {
  const [directory] = args;
  let previous;
  try {
    previous = JSON.parse(
      gh('release', 'view', tag, '--json', 'isDraft,assets'),
    );
  } catch (error) {
    // Only a missing release is a first assembly.
    if (!/release not found/.test(String(error.stderr))) throw error;
  }
  // A second run (a repeated tag push) must not re-sign a reviewed draft.
  draftGuard(previous, process.env.RELEASE_REASSEMBLE === 'true');
  if (!previous)
    gh(
      'release',
      'create',
      tag,
      '--draft',
      '--verify-tag',
      '--title',
      `Thelxinoe ${version}`,
      '--generate-notes',
    );
  const assets = readdirSync(directory)
    .filter(
      (name) =>
        /\.(exe|sig)$/.test(name) ||
        [
          'latest.json',
          'windows-x64.json',
          'release.pub',
          'compose.yaml',
          'env.example',
          'SHA256SUMS',
        ].includes(name),
    )
    .map((name) => resolve(directory, name));
  // Reassembly replaces the whole draft; no earlier asset may linger. The
  // completion marker leaves first and returns last, so an interrupted upload
  // never looks complete.
  const names = (previous?.assets ?? []).map(({ name }) => name);
  if (names.includes(completeDraftAsset))
    gh('release', 'delete-asset', tag, completeDraftAsset, '--yes');
  for (const name of names)
    if (
      name !== completeDraftAsset &&
      !assets.some((path) => basename(path) === name)
    )
      gh('release', 'delete-asset', tag, name, '--yes');
  const marker = resolve(directory, completeDraftAsset);
  gh(
    'release',
    'upload',
    tag,
    ...assets.filter((path) => path !== marker),
    '--clobber',
  );
  gh('release', 'upload', tag, marker);
} else if (command === 'publish') {
  const [requested, directory] = args;
  if (requested !== tag) throw Error('Tag and checkout version differ');
  const release = JSON.parse(gh('release', 'view', tag, '--json', 'isDraft'));
  const latest = latestRelease();
  // A retry after GitHub publication only finishes moving the `latest` tags.
  if (!release.isDraft && latest !== tag)
    throw Error('Only an existing reviewed draft can be published');
  if (release.isDraft) requireNewer(version, latest);
  checkSums(
    readFileSync(resolve(directory, 'SHA256SUMS'), 'utf8'),
    Object.fromEntries(
      readdirSync(directory)
        .filter((name) => name !== 'SHA256SUMS')
        .map((name) => [name, sha256(resolve(directory, name))]),
    ),
  );
  const commit = revision();
  const envelope = JSON.parse(
    readFileSync(resolve(directory, 'latest.json'), 'utf8'),
  );
  const manifest = JSON.parse(Buffer.from(envelope.payload, 'base64'));
  const installer = resolve(
    directory,
    basename(new URL(manifest.windows_x64.url).pathname),
  );
  verify(directory, installer);
  if (
    readFileSync(resolve(directory, 'release.pub'), 'utf8').trim() !==
      readFileSync('releases/release.pub', 'utf8').trim() ||
    readFileSync(resolve(directory, 'compose.yaml'), 'utf8').replaceAll(
      '\r\n',
      '\n',
    ) !== releaseCompose().replaceAll('\r\n', '\n') ||
    readFileSync(resolve(directory, 'env.example'), 'utf8').replaceAll(
      '\r\n',
      '\n',
    ) !== readFileSync('.env.example', 'utf8').replaceAll('\r\n', '\n')
  )
    throw Error('Deployment assets do not match this signed release');
  if (
    manifest.version !== version ||
    manifest.windows_x64.url !==
      `https://github.com/Valtrius/thelxinoe/releases/download/${tag}/${basename(installer)}`
  )
    throw Error('Draft identity does not match the release');
  const metadata = JSON.parse(
    readFileSync(resolve(directory, 'windows-x64.json'), 'utf8'),
  );
  if (
    metadata.version !== version ||
    metadata.platforms['windows-x86_64'].url !== manifest.windows_x64.url ||
    metadata.platforms['windows-x86_64'].signature !==
      manifest.windows_x64.signature
  )
    throw Error('Desktop metadata does not match the signed release');
  for (const component of ['server', 'controller']) {
    const value = manifest[component];
    const repository = `ghcr.io/valtrius/thelxinoe-${component}`;
    if (!value.reference.startsWith(repository + '@sha256:'))
      throw Error('Unexpected release registry');
    run('docker', ['pull', '--platform', 'linux/amd64', value.reference]);
    const [image] = JSON.parse(
      execFileSync('docker', ['inspect', value.reference], {
        encoding: 'utf8',
      }),
    );
    if (
      !image.RepoDigests.includes(value.reference) ||
      image.Os !== 'linux' ||
      image.Architecture !== 'amd64' ||
      image.Config.Labels['org.opencontainers.image.version'] !== version ||
      image.Config.Labels['org.opencontainers.image.revision'] !== commit
    )
      throw Error('Registry image identity mismatch');
  }
  // Deployments pull without credentials, so a private package would break every
  // install and update. Ask the registry the way an anonymous Docker host does.
  for (const component of ['server', 'controller']) {
    const [repository, digest] = manifest[component].reference.split('@');
    if ((await registryDigest(repository, digest)) !== digest)
      throw Error(
        `${repository} is not publicly pullable. On GitHub open Packages → thelxinoe-${component} → Package settings → Change visibility → Public, then run this workflow again.`,
      );
  }
  const promote = (reference, target) =>
    run('docker', [
      'buildx',
      'imagetools',
      'create',
      '--prefer-index=false',
      '--tag',
      target,
      reference,
    ]);
  // Version tags never move. `latest` moves last, so `docker compose pull`
  // only reaches a release whose signed feed is already public.
  for (const component of ['server', 'controller']) {
    const [repository, digest] = manifest[component].reference.split('@');
    const current = await registryDigest(repository, version);
    if (current && current !== digest)
      throw Error(`${repository}:${version} already names another image`);
    if (!current)
      promote(manifest[component].reference, `${repository}:${version}`);
  }
  if (release.isDraft) gh('release', 'edit', tag, '--draft=false', '--latest');
  for (const component of ['server', 'controller']) {
    const reference = manifest[component].reference;
    promote(reference, `${reference.split('@')[0]}:latest`);
  }
} else if (command === 'monitor') {
  // What every installation polls: the live signed feed and its images.
  const response = await fetch(feed);
  if (!response.ok) throw Error(`Release feed HTTP ${response.status}`);
  const manifest = verifyEnvelope(
    await response.json(),
    readFileSync('releases/release.pub', 'utf8'),
  );
  const days = checkValidity(manifest, Math.floor(Date.now() / 1000), 60);
  // An empty Docker configuration pulls as a host without credentials does.
  const anonymous = mkdtempSync(join(tmpdir(), 'thelxinoe-anonymous-'));
  for (const component of ['server', 'controller']) {
    const reference = manifest[component].reference;
    const [repository, digest] = reference.split('@');
    for (const name of ['latest', manifest.version]) {
      const found = await registryDigest(repository, name);
      if (found !== digest)
        throw Error(
          `${repository}:${name} resolves to ${found ?? 'nothing anonymously'}, but the feed names ${digest}`,
        );
    }
    run('docker', ['pull', '--platform', 'linux/amd64', reference], {
      env: { ...process.env, DOCKER_CONFIG: anonymous },
    });
    const [image] = JSON.parse(
      execFileSync('docker', ['image', 'inspect', reference], {
        encoding: 'utf8',
      }),
    );
    if (
      image.Config.Labels['org.opencontainers.image.version'] !==
      manifest.version
    )
      throw Error(`${reference} is not version ${manifest.version}`);
  }
  rmSync(anonymous, { recursive: true, force: true });
  console.log(
    `Release feed ${manifest.version} verified; valid for ${Math.floor(days)} more days`,
  );
} else if (command === 'smoke') {
  // Start the staged images with the shipped Compose file before signing them.
  const [serverRef, controllerRef] = args;
  if (!controllerRef) throw Error('smoke: server@digest controller@digest');
  for (const name of ['thelxinoe-server', 'thelxinoe-controller'])
    if (
      execFileSync('docker', ['ps', '-aq', '--filter', `name=^${name}$`], {
        encoding: 'utf8',
      }).trim()
    )
      throw Error(`${name} exists; run this on a disposable Docker host`);
  const root = mkdtempSync(join(tmpdir(), 'thelxinoe-smoke-'));
  const project = basename(root).toLowerCase();
  const env = {
    ...process.env,
    THELXINOE_SERVER_IMAGE: serverRef,
    THELXINOE_CONTROLLER_IMAGE: controllerRef,
    THELXINOE_STATE_ROOT: `${root}/state`,
    THELXINOE_MEDIA_ROOT: `${root}/media`,
    THELXINOE_BACKUP_ROOT: `${root}/backups`,
    THELXINOE_LISTEN: '127.0.0.1',
  };
  const compose = (...rest) =>
    run(
      'docker',
      [
        'compose',
        '-p',
        project,
        '-f',
        resolve('compose.release.yaml'),
        '--project-directory',
        root,
        ...rest,
      ],
      { env },
    );
  // Directories prepared as the deployment notes ask, for the default IDs.
  const prepare = (script) =>
    run('docker', [
      'run',
      '--rm',
      '--network',
      'none',
      '-v',
      `${root}:/smoke`,
      '--entrypoint',
      'sh',
      controllerRef,
      '-c',
      script,
    ]);
  try {
    prepare(
      'cd /smoke && mkdir -p state/server state/cache media backups && chown 10001:10001 state/server state/cache media backups',
    );
    compose('up', '-d', '--wait', '--wait-timeout', '300');
    const health = await (
      await fetch('http://127.0.0.1:8484/api/v1/health')
    ).json();
    const controller = JSON.parse(
      execFileSync(
        'docker',
        [
          'exec',
          'thelxinoe-controller',
          'curl',
          '-fsS',
          '--unix-socket',
          '/run/thelxinoe/controller.sock',
          'http://localhost/health',
        ],
        { encoding: 'utf8' },
      ),
    );
    if (health.version !== version || controller.version !== version)
      throw Error(
        `Started server ${health.version} and controller ${controller.version}, expected ${version}`,
      );
    console.log(`Release ${version} images started healthy with Compose`);
  } catch (error) {
    try {
      compose('logs', '--no-color');
    } catch {
      /* Report the original failure. */
    }
    throw error;
  } finally {
    compose('down', '--volumes', '--remove-orphans');
    prepare('rm -rf /smoke/state /smoke/media /smoke/backups');
    rmSync(root, { recursive: true, force: true });
  }
} else
  throw Error(
    'Usage: release.mjs check X.Y.Z | desktop | assemble ... | draft DIR | publish X.Y.Z DIR | monitor | smoke SERVER CONTROLLER',
  );
