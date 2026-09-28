// Assemble publication from final Windows bytes and immutable Linux image identities.
import {
  readFileSync,
  writeFileSync,
  mkdirSync,
  copyFileSync,
  readdirSync,
  unlinkSync,
} from 'node:fs';
import { execFileSync } from 'node:child_process';
import { resolve, basename } from 'node:path';
import { createPublicKey, createHash } from 'node:crypto';

const [command, ...args] = process.argv.slice(2);
const version = JSON.parse(readFileSync('package.json', 'utf8')).version;
const tag = version;
function releaseCompose(manifest) {
  return readFileSync('compose.release.yaml', 'utf8')
    .replace(
      /image: \$\{THELXINOE_SERVER_IMAGE:[^\n]+/,
      `image: ${manifest.server.reference}`,
    )
    .replace(
      /image: \$\{THELXINOE_CONTROLLER_IMAGE:[^\n]+/,
      `image: ${manifest.controller.reference}`,
    );
}
const run = (exe, args, options = {}) =>
  execFileSync(exe, args, { stdio: 'inherit', ...options });
const gh = (...args) => execFileSync('gh', args, { encoding: 'utf8' }).trim();
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
    if (!image.RepoDigests.includes(reference))
      throw Error('Pulled image does not match the registry digest');
    return { reference };
  }
  const schema = Number(
    readFileSync('crates/database/src/lib.rs', 'utf8').match(
      /SCHEMA_VERSION: u32 = (\d+)/,
    )[1],
  );
  const api = Number(
    readFileSync('crates/core/src/lib.rs', 'utf8').match(
      /API_VERSION: u32 = (\d+)/,
    )[1],
  );
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
    api: { min: api, max: api },
    migration: {
      from: { min: schema, max: schema },
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
  writeFileSync(resolve(output, 'compose.yaml'), releaseCompose(manifest));
  verify(output, resolve(output, file));
  const assets = [
    file,
    `${file}.sig`,
    'latest.json',
    'windows-x64.json',
    'release.pub',
    'compose.yaml',
  ];
  writeFileSync(
    resolve(output, 'SHA256SUMS'),
    assets
      .map(
        (name) =>
          `${createHash('sha256')
            .update(readFileSync(resolve(output, name)))
            .digest('hex')}  ${name}`,
      )
      .join('\n') + '\n',
  );
} else if (command === 'draft') {
  const [directory] = args;
  let previous;
  try {
    previous = JSON.parse(gh('release', 'view', tag, '--json', 'isDraft'));
  } catch {
    /* First assembly. */
  }
  if (previous && !previous.isDraft)
    throw Error(
      'Published releases are immutable; increment the product version',
    );
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
          'SHA256SUMS',
        ].includes(name),
    )
    .map((name) => resolve(directory, name));
  gh('release', 'upload', tag, ...assets, '--clobber');
} else if (command === 'publish') {
  const [requested, directory] = args;
  if (requested !== tag) throw Error('Tag and checkout version differ');
  const release = JSON.parse(gh('release', 'view', tag, '--json', 'isDraft'));
  if (!release.isDraft)
    throw Error('Only an existing reviewed draft can be published');
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
    ) !== releaseCompose(manifest).replaceAll('\r\n', '\n')
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
      image.Config.Labels['org.opencontainers.image.version'] !== version
    )
      throw Error('Registry image identity mismatch');
  }
  for (const component of ['server', 'controller']) {
    const repository = `ghcr.io/valtrius/thelxinoe-${component}`;
    run('docker', [
      'buildx',
      'imagetools',
      'create',
      '--prefer-index=false',
      '--tag',
      `${repository}:${version}`,
      '--tag',
      `${repository}:latest`,
      manifest[component].reference,
    ]);
  }
  gh('release', 'edit', tag, '--draft=false', '--latest');
} else throw Error('Usage: release.mjs check X.Y.Z | assemble ...');
