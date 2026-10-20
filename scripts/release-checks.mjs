// Pure release checks shared by scripts/release.mjs and its unit tests.
import { createPublicKey, verify } from 'node:crypto';

/** The feed every installation polls by default. */
export const feed =
  'https://github.com/Valtrius/thelxinoe/releases/latest/download/latest.json';

const parse = (version) => {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  if (!match) throw Error(`Invalid release version ${version}`);
  return match.slice(1).map(Number);
};
/** Whether `version` is a strictly newer X.Y.Z release than `other`. */
export function newer(version, other) {
  const [a, b] = [parse(version), parse(other)];
  const index = a.findIndex((part, i) => part !== b[i]);
  return index >= 0 && a[index] > b[index];
}
/** Publication moves the feed and `latest`, so it may only move forwards. */
export function requireNewer(version, latest) {
  if (latest && !newer(version, latest))
    throw Error(
      `${version} must be newer than the latest published release ${latest}`,
    );
}
/** Every downloaded asset matches SHA256SUMS, and every listed asset exists. */
export function checkSums(text, actual) {
  const listed = new Map();
  for (const line of text.split(/\r?\n/).filter(Boolean)) {
    const [, hash, name] = /^([a-f0-9]{64}) [ *](.+)$/.exec(line) ?? [];
    if (!name || listed.has(name))
      throw Error(`Invalid SHA256SUMS line: ${line}`);
    listed.set(name, hash);
  }
  for (const [name, hash] of listed)
    if (actual[name] !== hash)
      throw Error(`Release asset ${name} does not match SHA256SUMS`);
  for (const name of Object.keys(actual))
    if (!listed.has(name))
      throw Error(`Release asset ${name} is not listed in SHA256SUMS`);
}
/** Uploaded last, so only a complete draft has it. */
export const completeDraftAsset = 'SHA256SUMS';
/** A complete draft is kept unless reassembly is requested explicitly; an
 * interrupted upload may simply run again. */
export function draftGuard(release, reassemble) {
  if (!release) return;
  if (!release.isDraft)
    throw Error(
      'Published releases are immutable; increment the product version',
    );
  if (
    release.assets?.some(({ name }) => name === completeDraftAsset) &&
    !reassemble
  )
    throw Error(
      'This draft is already complete. Review or publish it; to replace it run Assemble release draft from the tag with reassemble enabled.',
    );
}
const DOMAIN = Buffer.from('Thelxinoe release manifest v1\0');
const SPKI = Buffer.from('302a300506032b6570032100', 'hex');
/** The manifest of an envelope signed by the base64 Ed25519 `publicKey`. */
export function verifyEnvelope(envelope, publicKey) {
  const raw = Buffer.from(publicKey.trim(), 'base64');
  if (raw.length !== 32) throw Error('Invalid release trust key');
  const key = createPublicKey({
    key: Buffer.concat([SPKI, raw]),
    format: 'der',
    type: 'spki',
  });
  const payload = Buffer.from(envelope.payload, 'base64');
  const signature = Buffer.from(envelope.signature, 'base64');
  if (!verify(null, Buffer.concat([DOMAIN, payload]), key, signature))
    throw Error('Release signature is invalid');
  return JSON.parse(payload);
}
/** A live feed must already be valid and stay valid for `days` more days. */
export function checkValidity(manifest, now, days) {
  if (manifest.published_at > now + 300)
    throw Error('The release feed is not valid yet');
  const left = (manifest.expires_at - now) / 86400;
  if (left < days)
    throw Error(
      `The release feed expires in ${Math.floor(left)} days; publish a renewed release`,
    );
  return left;
}
