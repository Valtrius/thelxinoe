// Release the current origin/develop head as X.Y.Z. Its version commit and tag are
// pushed together, so Assemble release draft builds while Build and verify runs.
import { readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { newer } from './release-checks.mjs';

const [version] = process.argv.slice(2);
const git = (...args) => execFileSync('git', args, { encoding: 'utf8' }).trim();
const node = (...args) =>
  execFileSync(process.execPath, args, { stdio: 'inherit' });
const current = JSON.parse(readFileSync('package.json', 'utf8')).version;
if (!version) throw Error('Usage: pnpm run release X.Y.Z');
if (!newer(version, current))
  throw Error(`${version} must be newer than ${current}`);
if (git('status', '--porcelain'))
  throw Error('Commit or discard local changes first');
git('fetch', '--quiet', 'origin', 'develop');
const base = git('rev-parse', 'HEAD');
if (base !== git('rev-parse', 'origin/develop'))
  throw Error('Check out the current origin/develop head first');

const cargo = readFileSync('Cargo.toml', 'utf8');
writeFileSync(
  'Cargo.toml',
  cargo.replace(
    /(\[workspace\.package\][\s\S]*?version = ")[^"]+/,
    `$1${version}`,
  ),
);
node('scripts/version.mjs', '--write');
node('scripts/release.mjs', 'check', version);
git('commit', '--quiet', '--all', '--message', `chore: release ${version}`);
git('tag', version);
try {
  execFileSync(
    'git',
    [
      'push',
      '--atomic',
      'origin',
      'HEAD:refs/heads/develop',
      `refs/tags/${version}`,
    ],
    { stdio: 'inherit' },
  );
} catch (error) {
  git('tag', '--delete', version);
  git('reset', '--keep', base);
  throw error;
}
console.log(
  `Releasing ${version}: approve its publish job once the draft is ready.\nhttps://github.com/Valtrius/thelxinoe/actions/workflows/release.yml`,
);
