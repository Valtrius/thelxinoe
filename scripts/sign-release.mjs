// Usage: node scripts/sign-release.mjs manifest.json private-key.pem envelope.json
// The private key is read from disk and is never printed.
import { readFileSync, writeFileSync } from 'node:fs';
import { sign, createPrivateKey } from 'node:crypto';
const [source, keyFile, output] = process.argv.slice(2);
if (!source || !keyFile || !output)
  throw Error(
    'Usage: sign-release.mjs manifest.json private-key.pem envelope.json',
  );
const payload = readFileSync(source);
JSON.parse(payload.toString('utf8'));
const key = createPrivateKey(readFileSync(keyFile));
if (key.asymmetricKeyType !== 'ed25519')
  throw Error('Release signing requires Ed25519');
const signature = sign(
  null,
  Buffer.concat([Buffer.from('Thelxinoe release manifest v1\0'), payload]),
  key,
);
writeFileSync(
  output,
  JSON.stringify(
    {
      payload: payload.toString('base64'),
      signature: signature.toString('base64'),
    },
    null,
    2,
  ) + '\n',
);
console.log('Signed release envelope written');
