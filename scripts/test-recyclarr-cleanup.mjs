import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';

const result = {
  started: new Date().toISOString(),
  finished: null,
  passed: false,
  assertions: [],
};
try {
  const child = spawnSync(
    process.execPath,
    ['scripts/test-recyclarr.mjs', '--interrupt-after-install'],
    { encoding: 'utf8', timeout: 600000, killSignal: 'SIGKILL' },
  );
  writeFileSync(
    '.local/recyclarr-cleanup.log',
    `${child.stdout ?? ''}${child.stderr ?? ''}`,
  );
  assert.ok(!child.error, 'Interrupted fixture must terminate on its own');
  assert.notEqual(child.status, 0, 'An interrupted fixture must fail');
  const fixture = JSON.parse(
    readFileSync('.local/recyclarr-result.json', 'utf8'),
  );
  result.fixture = fixture;
  assert.equal(fixture.scope, 'cleanup');
  assert.equal(fixture.passed, false);
  assert.ok(fixture.finished, 'A failed fixture must retain a finished result');
  assert.match(fixture.error, /Intentional fixture interruption/);
  assert.ok(
    fixture.cleanup_errors.some((error) => error.startsWith('browser trace:')),
  );
  for (const label of [
    `app.thelxinoe.deployment=${fixture.deployment}`,
    `com.docker.compose.project=${fixture.project}`,
  ]) {
    assert.equal(
      execFileSync('docker', ['ps', '-aq', '--filter', `label=${label}`], {
        encoding: 'utf8',
        timeout: 30000,
      }).trim(),
      '',
      `Interrupted fixture left containers for ${label}`,
    );
  }
  result.assertions.push(
    'Browser trace failure preserves a failed, finished fixture result',
    'Browser trace failure does not skip managed or Compose container cleanup',
  );
  result.passed = true;
  console.log('Recyclarr cleanup E2E passed');
} catch (error) {
  result.error = String(error);
  throw error;
} finally {
  result.finished = new Date().toISOString();
  writeFileSync(
    '.local/recyclarr-cleanup-result.json',
    JSON.stringify(result, null, 2),
  );
}
