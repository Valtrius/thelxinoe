import { execFileSync } from 'node:child_process';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { createHash, createPublicKey, generateKeyPairSync } from 'node:crypto';
import { createServer } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';

export const repository = resolve(import.meta.dirname, '../..');
export function run(command, args, options = {}) {
  return execFileSync(command, args, {
    cwd: repository,
    encoding: 'utf8',
    stdio: 'inherit',
    windowsHide: true,
    ...options,
  });
}
export const docker = (...args) =>
  run('docker', args, { stdio: ['ignore', 'pipe', 'pipe'] }).trim();
export const save = (path, value) =>
  writeFileSync(path, JSON.stringify(value, null, 2) + '\n');
export function certificate(lab) {
  const openssl =
    process.platform === 'win32' &&
    existsSync('C:/Program Files/Git/usr/bin/openssl.exe')
      ? 'C:/Program Files/Git/usr/bin/openssl.exe'
      : 'openssl';
  const file = (name) => join(lab.root, name);
  run(openssl, [
    'req',
    '-x509',
    '-newkey',
    'rsa:2048',
    '-nodes',
    '-days',
    '30',
    '-keyout',
    file('ca.key'),
    '-out',
    file('tls.pem'),
    '-subj',
    '/CN=Thelxinoe Update Lab CA',
    '-addext',
    'basicConstraints=critical,CA:TRUE',
    '-addext',
    'keyUsage=critical,keyCertSign,cRLSign',
  ]);
  run(openssl, [
    'req',
    '-new',
    '-newkey',
    'rsa:2048',
    '-nodes',
    '-keyout',
    file('tls.key'),
    '-out',
    file('tls.csr'),
    '-subj',
    '/CN=localhost',
  ]);
  writeFileSync(
    file('tls.ext'),
    'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,DNS:host.docker.internal,IP:127.0.0.1\n',
  );
  run(openssl, [
    'x509',
    '-req',
    '-in',
    file('tls.csr'),
    '-CA',
    file('tls.pem'),
    '-CAkey',
    file('ca.key'),
    '-CAcreateserial',
    '-out',
    file('tls.crt'),
    '-days',
    '30',
    '-extfile',
    file('tls.ext'),
  ]);
  mkdirSync(join(lab.root, 'trust'), { recursive: true });
  copyFileSync(file('tls.pem'), join(lab.root, 'trust/tls.pem'));
}
export function keys(lab) {
  const { privateKey } = generateKeyPairSync('ed25519');
  writeFileSync(
    join(lab.root, 'signing.pem'),
    privateKey.export({ type: 'pkcs8', format: 'pem' }),
    { mode: 0o600, flag: 'wx' },
  );
  const publicKey = createPublicKey(privateKey)
    .export({ type: 'spki', format: 'der' })
    .subarray(-32)
    .toString('base64');
  writeFileSync(join(lab.root, 'release.pub'), publicKey + '\n');
  certificate(lab);
  for (const file of ['release.pub', 'tls.pem'])
    copyFileSync(join(lab.root, file), join(lab.root, 'trust', file));
  if (lab.desktop) {
    run(process.execPath, [
      join(repository, 'node_modules/@tauri-apps/cli/tauri.js'),
      'signer',
      'generate',
      '-w',
      join(lab.root, 'updater.key'),
      '-p',
      '',
    ]);
  }
}
export function source(lab, version) {
  const directory = join(lab.root, 'source', version);
  mkdirSync(directory, { recursive: true });
  const files = run(
    'git',
    ['ls-files', '--cached', '--others', '--exclude-standard'],
    { stdio: ['ignore', 'pipe', 'pipe'] },
  )
    .trim()
    .split(/\r?\n/);
  for (const file of new Set(files)) {
    if (
      !/^(apps\/|crates\/|frontend\/|scripts\/|releases\/|Cargo\.|Dockerfile$|compose\.|package|\.dockerignore$)/.test(
        file,
      ) ||
      !existsSync(join(repository, file))
    )
      continue;
    const destination = join(directory, file);
    mkdirSync(dirname(destination), { recursive: true });
    copyFileSync(join(repository, file), destination);
  }
  const cargo = join(directory, 'Cargo.toml');
  writeFileSync(
    cargo,
    readFileSync(cargo, 'utf8').replace(
      /(\[workspace.package\][\s\S]*?version = ")[^"]+/,
      `$1${version}`,
    ),
  );
  run(process.execPath, [join(directory, 'scripts/version.mjs'), '--write'], {
    cwd: directory,
  });
  copyFileSync(
    join(lab.root, 'release.pub'),
    join(directory, 'releases/release.pub'),
  );
  const capabilityPath = join(
    directory,
    'apps/desktop/capabilities/default.json',
  );
  const capability = JSON.parse(readFileSync(capabilityPath, 'utf8'));
  capability.permissions.push(
    'core:window:allow-set-position',
    'core:window:allow-set-size',
  );
  save(capabilityPath, capability);
  // Fault points exist only in disposable source copies, never production.
  // Distinguish the live-state check from preflight copies, so a fault can be
  // armed before the one-click flow without failing the wrong phase.
  const controller = join(directory, 'apps/docker-controller/src/product.rs');
  writeFileSync(
    controller,
    readFileSync(controller, 'utf8').replace(
      '    let name = format!("thelxinoe-product-',
      `    let mut spec = spec;
    if label == "live-validation" {
        spec["Env"] = json!(["THELXINOE_LAB_LIVE_VALIDATION=1"]);
    }
    let name = format!("thelxinoe-product-`,
    ),
  );
  if (version === lab.next) {
    const validation = join(directory, 'apps/server/src/validation.rs');
    writeFileSync(
      validation,
      readFileSync(validation, 'utf8').replace(
        '    let report =',
        `
    if std::env::var_os("THELXINOE_LAB_LIVE_VALIDATION").is_some() && state.config.state.join("hold-live-validation").exists() {
        std::fs::write(state.config.state.join("held-validation-entered"), b"held")?;
        tokio::time::sleep(std::time::Duration::from_secs(120)).await;
    }
    if std::env::var_os("THELXINOE_LAB_LIVE_VALIDATION").is_some() && state.config.state.join("fail-live-validation").exists() {
        std::fs::write(state.config.state.join("failed-validation-mutated"), b"candidate wrote state")?;
        anyhow::bail!("Deliberate update lab validation failure");
    }
    let report =`,
      ),
    );
  }
  return directory;
}
export function containerBuild(lab, version, directory) {
  const images = {};
  for (const component of ['server', 'controller']) {
    const tag = `localhost:${lab.registryPort}/${lab.id}/${component}:${version}`;
    run('docker', [
      'build',
      '--target',
      component,
      '--build-arg',
      `VERSION=${version}`,
      '-t',
      tag,
      directory,
    ]);
    run('docker', ['push', tag]);
    const [image] = JSON.parse(docker('image', 'inspect', tag));
    images[component] = {
      reference: image.RepoDigests.find((v) =>
        v.startsWith(tag.split(':').slice(0, -1).join(':') + '@'),
      ),
    };
  }
  return images;
}
export async function nativeBuild(lab, version, directory) {
  // Cargo releases its lock before NSIS packaging reads the binary and writes
  // its shared staging files. Hold an OS-owned lock through artifact copying.
  const pipe = `\\\\.\\pipe\\thelxinoe-update-build-${createHash('sha256').update(repository.toLowerCase()).digest('hex').slice(0, 16)}`;
  let lock;
  let waiting = false;
  while (!lock) {
    const server = createServer();
    try {
      await new Promise((done, reject) => {
        server.once('error', reject);
        server.listen(pipe, done);
      });
      lock = server;
    } catch (error) {
      if (error.code !== 'EADDRINUSE') throw error;
      if (!waiting) console.log('Waiting for another lab desktop build');
      waiting = true;
      await delay(500);
    }
  }
  try {
    nativeArtifacts(lab, version, directory);
  } finally {
    await new Promise((done) => lock.close(done));
  }
}
function nativeArtifacts(lab, version, directory) {
  const dependencies = join(directory, 'node_modules');
  if (!existsSync(dependencies))
    symlinkSync(
      join(repository, 'node_modules'),
      dependencies,
      process.platform === 'win32' ? 'junction' : 'dir',
    );
  if (
    existsSync(join(repository, 'frontend/node_modules')) &&
    !existsSync(join(directory, 'frontend/node_modules'))
  )
    symlinkSync(
      join(repository, 'frontend/node_modules'),
      join(directory, 'frontend/node_modules'),
      process.platform === 'win32' ? 'junction' : 'dir',
    );
  const target = join(repository, 'target/update-lab');
  const config = join(lab.root, `desktop-${version}.json`);
  save(config, {
    identifier: lab.identifier,
    mainBinaryName: lab.id,
    productName: `Thelxinoe Update Lab ${lab.id.slice(-8)}`,
    bundle: { createUpdaterArtifacts: true },
    plugins: {
      updater: {
        pubkey: readFileSync(join(lab.root, 'updater.key.pub'), 'utf8').trim(),
        windows: { installMode: 'quiet' },
      },
    },
    app: {
      windows: [
        {
          title: 'Thelxinoe Update Lab',
          width: 1360,
          height: 900,
          visible: !lab.headless,
          decorations: false,
        },
      ],
    },
  });
  const env = {
    ...process.env,
    CARGO_TARGET_DIR: target,
    THELXINOE_RELEASE_PUBLIC_KEY: readFileSync(
      join(lab.root, 'release.pub'),
      'utf8',
    ).trim(),
    THELXINOE_RELEASE_URL: `${lab.publisher}/latest.json`,
    THELXINOE_RELEASE_CA_PEM: readFileSync(join(lab.root, 'tls.pem'), 'utf8'),
    TAURI_SIGNING_PRIVATE_KEY: readFileSync(
      join(lab.root, 'updater.key'),
      'utf8',
    ),
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: '',
  };
  run(
    process.execPath,
    [
      join(repository, 'node_modules/@tauri-apps/cli/tauri.js'),
      'icon',
      join(directory, 'frontend/public/icon.svg'),
      '--output',
      join(directory, 'apps/desktop/icons'),
    ],
    { cwd: directory },
  );
  run(
    process.execPath,
    [
      join(repository, 'node_modules/@tauri-apps/cli/tauri.js'),
      'build',
      '--config',
      config,
    ],
    { cwd: join(directory, 'apps/desktop'), env },
  );
  const bundle = join(target, 'release/bundle/nsis');
  const installer = `Thelxinoe Update Lab ${lab.id.slice(-8)}_${version}_x64-setup.exe`;
  if (!existsSync(join(bundle, installer)))
    throw Error(`Missing ${version} lab installer`);
  const release = join(lab.root, 'channel', version);
  mkdirSync(release, { recursive: true });
  copyFileSync(join(bundle, installer), join(release, 'setup.exe'));
  copyFileSync(
    join(bundle, `${installer}.sig`),
    join(release, 'setup.exe.sig'),
  );
  if (version === lab.base && !lab.server) {
    run('cargo', ['build', '--release', '-p', 'thelxinoe-server'], {
      cwd: directory,
      env,
    });
    copyFileSync(
      join(target, 'release/thelxinoe-server.exe'),
      join(lab.root, 'server.exe'),
    );
  }
}
export function envelope(lab, version, images) {
  const release = join(lab.root, 'channel', version);
  mkdirSync(release, { recursive: true });
  if (!lab.desktop) {
    writeFileSync(
      join(release, 'setup.exe'),
      'Unused Linux server qualification fixture',
    );
    writeFileSync(join(release, 'setup.exe.sig'), 'unused-server-fixture');
    writeFileSync(join(lab.root, 'updater.key.pub'), 'unused-server-fixture');
  }
  const image = {
    reference: `example.invalid/update-lab@sha256:${'a'.repeat(64)}`,
  };
  const schema = Number(
    readFileSync(join(repository, 'crates/database/src/lib.rs'), 'utf8').match(
      /SCHEMA_VERSION: u32 = (\d+)/,
    )[1],
  );
  const api = Number(
    readFileSync(join(repository, 'crates/core/src/lib.rs'), 'utf8').match(
      /API_VERSION: u32 = (\d+)/,
    )[1],
  );
  const now = Math.floor(Date.now() / 1000);
  save(join(release, 'draft.json'), {
    format: 1,
    version,
    published_at: now - 60,
    expires_at: now + 30 * 86400,
    server: images?.server ?? image,
    controller: images?.controller ?? image,
    windows_x64: { url: `${lab.publisher}/releases/${version}/setup.exe` },
    api: { min: api, max: api },
    migration: {
      from: { min: schema, max: schema },
      target: schema,
      recovery: 'full-state-restore',
      recovery_protocol: 1,
    },
    notes: `Local update ${version}. This release uses disposable keys and installation state.`,
  });
  run(process.execPath, [
    'scripts/assemble-release.mjs',
    join(release, 'draft.json'),
    join(release, 'setup.exe'),
    join(release, 'setup.exe.sig'),
    join(lab.root, 'updater.key.pub'),
    join(release, 'manifest.json'),
  ]);
  run(process.execPath, [
    'scripts/sign-release.mjs',
    join(release, 'manifest.json'),
    join(lab.root, 'signing.pem'),
    join(release, 'latest.json'),
  ]);
  if (lab.desktop)
    run(
      'cargo',
      [
        'run',
        '--release',
        '--locked',
        '-p',
        'thelxinoe-releases',
        '--bin',
        'verify-release',
        '--',
        join(release, 'latest.json'),
        join(lab.root, 'release.pub'),
        join(release, 'setup.exe'),
      ],
      {
        cwd: lab.sources[version],
        env: {
          ...process.env,
          CARGO_TARGET_DIR: join(repository, 'target/update-lab'),
        },
      },
    );
}
