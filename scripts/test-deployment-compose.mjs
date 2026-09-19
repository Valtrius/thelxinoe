import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const root = '.local/managed-compose-proof';
mkdirSync(root, { recursive: true });
for (const name of [
  'compose.yaml',
  'compose.override.yaml',
  'desired-state.json',
]) {
  const bytes = execFileSync('docker', [
    'compose',
    '-f',
    'compose.managed.test.yaml',
    'exec',
    '-T',
    'controller',
    'cat',
    '/var/lib/thelxinoe/deployment/' + name,
  ]);
  writeFileSync(root + '/' + name, bytes);
}
const images = execFileSync(
  'docker',
  ['compose', '--project-directory', root, 'config', '--images'],
  { encoding: 'utf8' },
);
const descriptor = JSON.parse(readFileSync(root + '/desired-state.json'));
const accepted = [descriptor.server.Image, descriptor.controller.Image].sort();
assert.deepEqual(images.trim().split(/\r?\n/).sort(), accepted);
// Simulate an older bootstrap file beside the authoritative override.
writeFileSync(
  root + '/compose.yaml',
  JSON.stringify({
    services: {
      server: { image: 'thelxinoe-server:stale-bootstrap' },
      controller: { image: 'thelxinoe-controller:stale-bootstrap' },
    },
  }),
);
const recovered = execFileSync(
  'docker',
  ['compose', '--project-directory', root, 'config', '--images'],
  { encoding: 'utf8' },
);
assert.deepEqual(recovered.trim().split(/\r?\n/).sort(), accepted);
console.log('Accepted immutable images override stale bootstrap images.');
