import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { writeFileSync, openSync, closeSync } from 'node:fs';
import {
  fixtureId,
  freePort,
  resourceRecord,
  resourceScope,
} from './ci-resources.mjs';
import { imageManifest, pullFixtureImage } from './ci-images.mjs';
import { waitForState } from './ci-readiness.mjs';

const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    windowsHide: true,
    timeout: 30000,
    stdio: 'pipe',
  }).trim();
export async function startBrowser({ resourceDirectory } = {}) {
  resourceScope();
  const require = createRequire(import.meta.url);
  const version = require('@playwright/test/package.json').version;
  const alias = `mcr.microsoft.com/playwright:v${version}-noble`;
  if (!imageManifest.infrastructure[alias])
    throw Error(
      `Pin the Playwright ${version} image in scripts/ci-images.json`,
    );
  const image = await pullFixtureImage(alias);
  const name = fixtureId('browser');
  const runtime = `${name}:${process.env.THELXINOE_CI_RUN_ID}`;
  const port = await freePort();
  const record = resourceRecord(
    {
      containers: [name],
      images: [runtime],
      closed: false,
    },
    resourceDirectory,
  );
  const close = () => {
    try {
      writeFileSync('.local/ci-browser.log', docker('logs', name));
    } catch {
      /* Startup may have failed before the container existed. */
    }
    record.close();
  };
  try {
    const log = openSync('.local/ci-browser-build.log', 'w');
    try {
      execFileSync(
        'docker',
        [
          'build',
          '-f',
          'scripts/Dockerfile.browser',
          '--build-arg',
          `BASE_IMAGE=${image}`,
          '--build-arg',
          `PLAYWRIGHT_VERSION=${version}`,
          '-t',
          runtime,
          '.',
        ],
        {
          timeout: 300000,
          encoding: 'utf8',
          windowsHide: true,
          stdio: ['ignore', log, log],
        },
      );
    } finally {
      closeSync(log);
    }
    record({
      imageIds: {
        [runtime]: docker('image', 'inspect', '--format', '{{.Id}}', runtime),
      },
    });
    docker(
      'run',
      '--detach',
      '--pull',
      'never',
      '--name',
      name,
      '--label',
      `io.thelxinoe.ci-run=${process.env.THELXINOE_CI_RUN_ID}`,
      '--init',
      '--ipc=host',
      '--user',
      'pwuser',
      '--workdir',
      '/opt/playwright',
      '--publish',
      `127.0.0.1:${port}:3000`,
      runtime,
    );
    const endpoint = await waitForState(
      'Isolated browser startup',
      async () => docker('logs', name).match(/ws:\/\/[^\s]+/)?.[0],
      (value) => !!value,
      { timeout: 30000, interval: 500 },
    );
    const address = new URL(endpoint);
    address.hostname = '127.0.0.1';
    address.port = String(port);
    process.env.THELXINOE_CI_BROWSER_WS_ENDPOINT = address.href;
    return { close, image, runtime, version, endpoint: address.href };
  } catch (error) {
    try {
      close();
    } catch (cleanup) {
      throw new AggregateError(
        [error, cleanup],
        'Browser startup and cleanup failed',
        { cause: cleanup },
      );
    }
    throw error;
  }
}
