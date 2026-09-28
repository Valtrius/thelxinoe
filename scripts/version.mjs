import { readFileSync, writeFileSync } from 'node:fs';
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
for (const file of [
  'compose.yaml',
  'compose.test.yaml',
  'compose.release.yaml',
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
const lock = JSON.parse(readFileSync('package-lock.json', 'utf8'));
const lockVersions = [lock, lock.packages[''], lock.packages.frontend];
if (lockVersions.some((entry) => entry.version !== version)) {
  if (!process.argv.includes('--write'))
    throw new Error('package-lock.json: version is out of sync');
  for (const entry of lockVersions) entry.version = version;
  writeFileSync('package-lock.json', JSON.stringify(lock, null, 2) + '\n');
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
console.log(`Product version ${version}`);
