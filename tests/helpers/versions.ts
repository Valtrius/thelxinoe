import { readFileSync } from 'node:fs';

// Fixtures run the product version the web client was built as, so update
// flows compare against the real client, and offer its next patch release.
export const current: string = JSON.parse(
  readFileSync('package.json', 'utf8'),
).version;
const [major, minor, patch] = current.split('.').map(Number);
export const next = `${major}.${minor}.${patch + 1}`;
