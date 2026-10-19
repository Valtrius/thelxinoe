import { test } from 'node:test';
import assert from 'node:assert/strict';
import { generateKeyPairSync, sign } from 'node:crypto';
import {
  newer,
  requireNewer,
  checkSums,
  draftGuard,
  verifyEnvelope,
  checkValidity,
} from './release-checks.mjs';

test('versions compare numerically and only strictly newer releases publish', () => {
  assert.equal(newer('0.0.3', '0.0.2'), true);
  assert.equal(newer('0.0.10', '0.0.9'), true);
  assert.equal(newer('0.1.0', '0.0.99'), true);
  assert.equal(newer('1.0.0', '0.9.9'), true);
  assert.equal(newer('0.0.2', '0.0.2'), false);
  assert.equal(newer('0.0.2', '0.0.3'), false);
  assert.throws(() => newer('0.0.3-alpha.1', '0.0.2'));
  requireNewer('0.0.3', null);
  requireNewer('0.0.3', '0.0.2');
  assert.throws(() => requireNewer('0.0.2', '0.0.2'), /newer than/);
  assert.throws(() => requireNewer('0.0.1', '0.0.2'), /newer than/);
});

test('SHA256SUMS must list exactly the downloaded assets', () => {
  const [a, b] = ['a'.repeat(64), 'b'.repeat(64)];
  const sums = `${a}  latest.json\r\n${b}  compose.yaml\n`;
  checkSums(sums, { 'latest.json': a, 'compose.yaml': b });
  checkSums(`${a} *latest.json\n`, { 'latest.json': a });
  assert.throws(
    () => checkSums(sums, { 'latest.json': b, 'compose.yaml': b }),
    /latest\.json does not match/,
  );
  assert.throws(
    () => checkSums(sums, { 'latest.json': a }),
    /compose\.yaml does not match/,
  );
  assert.throws(
    () =>
      checkSums(sums, { 'latest.json': a, 'compose.yaml': b, 'old.exe': a }),
    /old\.exe is not listed/,
  );
  assert.throws(
    () => checkSums(`${a}  x\n${b}  x\n`, { x: a }),
    /Invalid SHA256SUMS/,
  );
  assert.throws(() => checkSums('garbage\n', {}), /Invalid SHA256SUMS/);
});

test('complete drafts are only replaced on explicit reassembly', () => {
  draftGuard(undefined, false);
  // An interrupted upload may already hold the signed feed.
  draftGuard(
    {
      isDraft: true,
      assets: [{ name: 'release.pub' }, { name: 'latest.json' }],
    },
    false,
  );
  const complete = {
    isDraft: true,
    assets: [{ name: 'latest.json' }, { name: 'SHA256SUMS' }],
  };
  assert.throws(() => draftGuard(complete, false), /reassemble/);
  draftGuard(complete, true);
  assert.throws(
    () => draftGuard({ isDraft: false, assets: [] }, true),
    /immutable/,
  );
});

test('envelopes verify against the trust key and its signing domain', () => {
  const { privateKey, publicKey } = generateKeyPairSync('ed25519');
  const key = publicKey
    .export({ type: 'spki', format: 'der' })
    .subarray(-32)
    .toString('base64');
  const payload = Buffer.from(JSON.stringify({ version: '0.0.3' }));
  const envelope = {
    payload: payload.toString('base64'),
    signature: sign(
      null,
      Buffer.concat([Buffer.from('Thelxinoe release manifest v1\0'), payload]),
      privateKey,
    ).toString('base64'),
  };
  assert.equal(verifyEnvelope(envelope, key + '\n').version, '0.0.3');
  const tampered = {
    ...envelope,
    payload: Buffer.from('{"version":"0.0.4"}').toString('base64'),
  };
  assert.throws(() => verifyEnvelope(tampered, key), /signature is invalid/);
  const other = generateKeyPairSync('ed25519')
    .publicKey.export({ type: 'spki', format: 'der' })
    .subarray(-32)
    .toString('base64');
  assert.throws(() => verifyEnvelope(envelope, other), /signature is invalid/);
  assert.throws(() => verifyEnvelope(envelope, 'c2hvcnQ='), /trust key/);
  const undomained = {
    ...envelope,
    signature: sign(null, payload, privateKey).toString('base64'),
  };
  assert.throws(() => verifyEnvelope(undomained, key), /signature is invalid/);
});

test('live feeds need 60 days of remaining validity', () => {
  const now = 1_800_000_000;
  const manifest = (days) => ({
    published_at: now - 86400,
    expires_at: now + days * 86400,
  });
  assert.equal(checkValidity(manifest(61), now, 60), 61);
  assert.throws(() => checkValidity(manifest(59), now, 60), /expires in 59/);
  assert.throws(
    () =>
      checkValidity({ published_at: now + 3600, expires_at: now * 2 }, now, 60),
    /not valid yet/,
  );
});
