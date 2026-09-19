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
for (const file of ['compose.yaml', 'compose.test.yaml']) {
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
console.log(`Product version ${version}`);
