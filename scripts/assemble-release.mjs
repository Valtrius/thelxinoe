// Populate the desktop portion of a reviewed release manifest with actual build evidence.
// Usage: assemble-release.mjs draft.json installer.exe installer.exe.sig updater.pub output.json
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join } from 'node:path';
const [source, artifact, signature, publicKey, output] = process.argv.slice(2);
if (!output)
  throw Error(
    'Usage: assemble-release.mjs draft.json installer.exe installer.exe.sig updater.pub output.json',
  );
const manifest = JSON.parse(readFileSync(source, 'utf8'));
const bytes = readFileSync(artifact);
const url = new URL(manifest.windows_x64.url);
if (url.protocol !== 'https:' || url.username || url.password)
  throw Error('Artifact URL must be HTTPS without credentials');
manifest.windows_x64 = {
  url: url.href,
  sha256: createHash('sha256').update(bytes).digest('hex'),
  bytes: bytes.length,
  signature: readFileSync(signature, 'utf8').trim(),
  updater_public_key: readFileSync(publicKey, 'utf8').trim(),
};
writeFileSync(output, JSON.stringify(manifest, null, 2) + '\n');
// Publish this sibling alongside the installer and signed release envelope.
writeFileSync(
  join(dirname(output), 'windows-x64.json'),
  JSON.stringify(
    {
      version: manifest.version,
      notes: manifest.notes,
      pub_date: new Date(manifest.published_at * 1000).toISOString(),
      platforms: {
        'windows-x86_64': {
          url: manifest.windows_x64.url,
          signature: manifest.windows_x64.signature,
        },
      },
    },
    null,
    2,
  ) + '\n',
);
console.log('Release hashes and desktop updater metadata assembled');
