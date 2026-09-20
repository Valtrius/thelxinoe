import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createHash, createPublicKey, generateKeyPairSync } from 'node:crypto';
import { execFileSync } from 'node:child_process';
for (const dir of ['channel', 'server', 'cache', 'data'])
  mkdirSync(`.local/releases/${dir}`, { recursive: true });
const keyFile = '.local/releases/signing.pem';
if (!existsSync(keyFile)) {
  const { privateKey } = generateKeyPairSync('ed25519');
  writeFileSync(keyFile, privateKey.export({ type: 'pkcs8', format: 'pem' }), {
    mode: 0o600,
    flag: 'wx',
  });
}
const publicKey = createPublicKey(readFileSync(keyFile))
  .export({ type: 'spki', format: 'der' })
  .subarray(-32)
  .toString('base64');
writeFileSync('.local/releases/channel/release.pub', publicKey + '\n');
function image(kind) {
  const repo = `localhost:25000/thelxinoe/${kind}`;
  const result = JSON.parse(
    execFileSync('docker', ['image', 'inspect', `${repo}:0.2.0`], {
      encoding: 'utf8',
    }),
  )[0];
  const reference = result.RepoDigests.find((s) => s.startsWith(repo + '@'));
  if (!reference)
    throw Error('Push the fixture images to the local test registry first');
  return { reference, config_digest: result.Id };
}
const installer = readFileSync(
  'target/release/bundle/nsis/Thelxinoe_0.1.0_x64-setup.exe',
);
const now = Math.floor(Date.now() / 1000);
const manifest = {
  format: 1,
  version: '0.2.0',
  published_at: now - 60,
  expires_at: now + 86400 * 7,
  server: image('server'),
  controller: image('controller'),
  windows_x64: {
    url: 'https://example.invalid/fixture.exe',
    sha256: createHash('sha256').update(installer).digest('hex'),
    bytes: installer.length,
    signature: 'controller-fixture-only',
    updater_public_key: 'controller-fixture-only',
  },
  api: { min: 1, max: 1 },
  migration: {
    from: { min: 25, max: 25 },
    target: 26,
    recovery: 'full-state-restore',
    recovery_protocol: 1,
  },
  notes:
    'Private release fixture: isolated migration, offline recovery and fenced handoff.',
};
const native =
  existsSync('.local/releases/channel/setup.exe') &&
  existsSync('.local/releases/channel/setup.exe.sig');
if (native) manifest.windows_x64.url = 'https://localhost:29443/setup.exe';
writeFileSync(
  '.local/releases/manifest.json',
  JSON.stringify(manifest, null, 2) + '\n',
);
if (native)
  execFileSync(
    process.execPath,
    [
      'scripts/assemble-release.mjs',
      '.local/releases/manifest.json',
      '.local/releases/channel/setup.exe',
      '.local/releases/channel/setup.exe.sig',
      '.local/releases/tauri.key.pub',
      '.local/releases/channel/manifest.json',
    ],
    { stdio: 'inherit' },
  );
execFileSync(
  process.execPath,
  [
    'scripts/sign-release.mjs',
    native
      ? '.local/releases/channel/manifest.json'
      : '.local/releases/manifest.json',
    keyFile,
    '.local/releases/channel/latest.json',
  ],
  { stdio: 'inherit' },
);
