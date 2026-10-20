import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
const cargo = readFileSync('Cargo.toml', 'utf8');
const version = cargo.match(
  /\[workspace.package\][\s\S]*?version = "([^"]+)"/,
)[1];
for (const file of [
  'package.json',
  'frontend/package.json',
  'apps/desktop/tauri.conf.json',
]) {
  const data = JSON.parse(readFileSync(file, 'utf8'));
  if (data.version !== version) {
    if (process.argv.includes('--write')) {
      data.version = version;
      writeFileSync(file, JSON.stringify(data, null, 2) + '\n');
    } else
      throw new Error(`${file}: expected shared product version ${version}`);
  }
}
// The release compose file follows `latest` and needs no version.
for (const file of [
  'compose.yaml',
  'compose.test.yaml',
  'compose.acquisition.test.yaml',
  'compose.managed.test.yaml',
  'compose.operations.test.yaml',
  'compose.segments.test.yaml',
  'scripts/generate-segment-fixtures.mjs',
]) {
  const text = readFileSync(file, 'utf8');
  const next = text.replace(
    /thelxinoe-(server|controller):\d+\.\d+\.\d+(?:-[\w.]+)?/g,
    `thelxinoe-$1:${version}`,
  );
  if (text !== next) {
    if (process.argv.includes('--write')) writeFileSync(file, next);
    else throw new Error(`${file}: expected shared product version ${version}`);
  }
}
const rustLock = readFileSync('Cargo.lock', 'utf8');
const nextLock = rustLock.replace(
  /(name = "thelxinoe-[^"]+"\r?\nversion = ")[^"]+/g,
  `$1${version}`,
);
if (rustLock !== nextLock) {
  if (!process.argv.includes('--write'))
    throw new Error('Cargo.lock: product versions are out of sync');
  writeFileSync('Cargo.lock', nextLock);
}
const dockerfile = readFileSync('Dockerfile', 'utf8');
const nextDockerfile = dockerfile.replace(
  /^ARG VERSION=.+$/gm,
  `ARG VERSION=${version}`,
);
if (dockerfile !== nextDockerfile) {
  if (!process.argv.includes('--write'))
    throw new Error('Dockerfile: product version is out of sync');
  writeFileSync('Dockerfile', nextDockerfile);
}
// A release ships the current database schema, which then never changes: pin its digest.
const schema = readFileSync('crates/database/src/lib.rs', 'utf8').match(
  /pub const SCHEMA_VERSION: u32 = (\d+)/,
)[1];
const pins = 'crates/database/schemas/SHA256SUMS';
const pinned = readFileSync(pins, 'utf8');
if (
  process.argv.includes('--write') &&
  !pinned.split(/\r?\n/).some((line) => line.endsWith(`  ${schema}.sql`))
) {
  const digest = createHash('sha256')
    .update(
      readFileSync(`crates/database/schemas/${schema}.sql`, 'utf8').replace(
        /\r\n/g,
        '\n',
      ),
    )
    .digest('hex');
  writeFileSync(pins, `${pinned}${digest}  ${schema}.sql\n`);
}
console.log(`Product version ${version}`);
