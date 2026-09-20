import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { setTimeout } from 'node:timers/promises';
const project = process.env.THELXINOE_RELEASE_PROJECT || 'thelxinoe-release-v6';
const root = `.local/${project}-compose`;
const controller = `${project}-controller-1`;
const run = (args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
mkdirSync(root, { recursive: true });
for (const file of [
  'compose.yaml',
  'compose.override.yaml',
  'desired-state.json',
]) {
  writeFileSync(
    `${root}/${file}`,
    run(['exec', controller, 'cat', `/var/lib/thelxinoe/deployment/${file}`]),
  );
}
const before = JSON.parse(readFileSync(`${root}/desired-state.json`));
writeFileSync(
  `${root}/compose.yaml`,
  JSON.stringify({
    name: project,
    services: {
      server: { image: 'thelxinoe-server:stale-bootstrap' },
      controller: { image: 'thelxinoe-controller:stale-bootstrap' },
    },
  }),
);
const config = JSON.parse(
  run(['compose', '--project-directory', root, 'config', '--format', 'json']),
);
assert.equal(config.services.server.image, before.server.Image);
assert.equal(config.services.controller.image, before.controller.Image);
try {
  run([
    'compose',
    '--project-directory',
    root,
    'up',
    '-d',
    '--force-recreate',
    '--no-build',
    '--pull',
    'never',
  ]);
  let health;
  for (let n = 0; n < 90; n++) {
    try {
      health = JSON.parse(
        run([
          'exec',
          controller,
          'curl',
          '-fsS',
          '--unix-socket',
          '/run/thelxinoe/controller.sock',
          'http://localhost/health',
        ]),
      );
      break;
    } catch {
      await setTimeout(1000);
    }
  }
  assert.equal(
    health?.version,
    before.version,
    'Pinned recreated controller must become healthy',
  );
  const after = JSON.parse(
    run([
      'exec',
      controller,
      'cat',
      '/var/lib/thelxinoe/deployment/desired-state.json',
    ]),
  );
  assert.ok(after.generation > before.generation);
  assert.equal(after.server.Image, before.server.Image);
  assert.equal(after.controller.Image, before.controller.Image);
  assert.notEqual(after.server.Id, before.server.Id);
  assert.notEqual(after.controller.Id, before.controller.Id);
  writeFileSync(
    `${root}/result.json`,
    JSON.stringify({
      passed: true,
      staleBootstrapIgnored: true,
      recreatedGenerationAccepted: true,
    }),
  );
  console.log(
    'Stale bootstrap override and actual pinned container recreation passed',
  );
} catch {
  throw Error(
    'Pinned deployment recreation failed; inspect the private test deployment',
  );
}
