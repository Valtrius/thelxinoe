import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, mkdirSync } from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';
import { jsonWrite } from './ci-state.mjs';

const bytes = readFileSync(new URL('./ci-images.json', import.meta.url));
export const imageManifest = JSON.parse(bytes);
export const imageManifestHash = createHash('sha256')
  .update(bytes)
  .digest('hex');
for (const image of [
  ...Object.values(imageManifest.services),
  ...Object.values(imageManifest.infrastructure),
])
  if (!/^[-\w./:]+@sha256:[a-f0-9]{64}$/.test(image))
    throw Error(`CI image is not pinned: ${image}`);

export const fixtureImage = (reference) =>
  imageManifest.infrastructure[reference] ?? reference;

function failureCategory(message) {
  if (/unauthorized|denied|authentication|forbidden/i.test(message))
    return 'authorization';
  if (/manifest unknown|not found|no matching manifest/i.test(message))
    return 'missing-manifest';
  if (/429|toomanyrequests|rate limit/i.test(message)) return 'rate-limit';
  if (/no space|disk full|ENOSPC/i.test(message)) return 'storage';
  if (/timed? ?out|ETIMEDOUT/i.test(message)) return 'timeout';
  return 'transport';
}

export async function pullFixtureImage(reference) {
  const image = fixtureImage(reference);
  const evidence = [];
  mkdirSync('.local', { recursive: true });
  const save = () =>
    jsonWrite('.local/ci-image-staging.json', {
      manifest_sha256: imageManifestHash,
      images: staged,
      last_attempts: evidence,
    });
  const started = Date.now();
  try {
    const raw = JSON.parse(
      execFileSync('docker', ['image', 'inspect', image], {
        encoding: 'utf8',
        windowsHide: true,
        stdio: 'pipe',
      }),
    )[0];
    staged[image] = { id: raw.Id, cached: true };
    save();
    return image;
  } catch {
    // An exact digest cache hit avoids registry access; a miss is staged below.
  }
  for (let attempt = 1; attempt <= 4; attempt++) {
    try {
      console.log(`Staging ${image} (attempt ${attempt})`);
      execFileSync('docker', ['pull', image], {
        timeout: Math.min(120000, 300000 - (Date.now() - started)),
        encoding: 'utf8',
        windowsHide: true,
        stdio: 'pipe',
        maxBuffer: 16 * 1024 * 1024,
      });
      const raw = JSON.parse(
        execFileSync('docker', ['image', 'inspect', image], {
          encoding: 'utf8',
          windowsHide: true,
        }),
      )[0];
      staged[image] = {
        id: raw.Id,
        cached: false,
        attempts: attempt,
        elapsed_ms: Date.now() - started,
      };
      save();
      return image;
    } catch (error) {
      const category = failureCategory(
        `${error.code ?? ''} ${error.stderr ?? ''}`,
      );
      evidence.push({
        image,
        attempt,
        category,
        exit_code: error.status ?? null,
        elapsed_ms: Date.now() - started,
      });
      save();
      if (
        ['authorization', 'missing-manifest', 'storage'].includes(category) ||
        attempt === 4 ||
        Date.now() - started >= 290000
      )
        throw Error(
          `Image staging failed: ${image} (${category}); see .local/ci-image-staging.json`,
          { cause: error },
        );
      await delay(1000 * 3 ** (attempt - 1));
    }
  }
}
const staged = {};
export async function stageBuildImages(
  images = [
    'node:24-bookworm-slim',
    'rust:1.98-bookworm',
    'debian:bookworm-slim',
    'python:3.11-slim-trixie',
  ],
) {
  for (const image of images) await pullFixtureImage(image);
}
export async function stageServiceImages() {
  await stageBuildImages();
  for (const image of Object.values(imageManifest.services))
    await pullFixtureImage(image);
  await pullFixtureImage('caddy:2');
}
