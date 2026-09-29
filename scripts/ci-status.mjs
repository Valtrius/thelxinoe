import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const directory = process.argv[2]
  ? resolve(process.argv[2])
  : JSON.parse(readFileSync(join(root, '.local/ci/latest.json'), 'utf8'))
      .directory;
console.log(readFileSync(join(directory, 'summary.txt'), 'utf8'));
console.log(`Summary: ${join(directory, 'index.html')}`);
console.log(`Result: ${join(directory, 'result.json')}`);
