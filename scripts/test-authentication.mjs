import { qualifyHostRecovery } from './auth-host-recovery.mjs';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { composeFixture, fixtureId, freePort } from './ci-resources.mjs';

const project = fixtureId('authentication');
process.env.THELXINOE_TEST_HTTP_PORT = String(await freePort({ udp: true }));
process.env.THELXINOE_TEST_HTTPS_PORT = String(await freePort());
process.env.THELXINOE_TEST_OIDC_PORT = String(await freePort());
const fixture = composeFixture({
  project,
  file: 'compose.test.yaml',
  root: `.local/${project}`,
});
try {
  fixture.compose('up', '-d', '--wait');
  execFileSync(
    process.execPath,
    [
      resolve('node_modules/@playwright/test/cli.js'),
      'test',
      '--project=authentication',
    ],
    {
      stdio: 'inherit',
      windowsHide: true,
      env: {
        ...process.env,
        THELXINOE_PROXY_TEST: '1',
        THELXINOE_TEST_URL: `https://localhost:${process.env.THELXINOE_TEST_HTTPS_PORT}`,
      },
    },
  );
  await qualifyHostRecovery(fixture, 'test-results/e2e/host-recovery.json');
} finally {
  try {
    writeFileSync(
      `.local/${project}/services.log`,
      fixture.compose('logs', '--no-color'),
    );
  } finally {
    fixture.close();
  }
}
