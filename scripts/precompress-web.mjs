// Writes `.br` and `.gz` copies of a web build's text files for the server to
// send as they are. Only the server image runs this: the desktop app bundles
// the build itself and would only grow.
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { brotliCompressSync, constants, gzipSync } from 'node:zlib';

const root = process.argv[2];
if (!root) throw new Error('Usage: node scripts/precompress-web.mjs <dir>');
const text = /\.(?:html|js|mjs|css|svg|json|webmanifest|txt|xml)$/;
let written = 0;
for (const entry of await readdir(root, {
  recursive: true,
  withFileTypes: true,
})) {
  if (!entry.isFile() || !text.test(entry.name)) continue;
  const path = join(entry.parentPath, entry.name);
  const source = await readFile(path);
  if (source.length < 1024) continue;
  for (const [extension, encoded] of [
    [
      '.br',
      brotliCompressSync(source, {
        params: {
          [constants.BROTLI_PARAM_QUALITY]: constants.BROTLI_MAX_QUALITY,
          [constants.BROTLI_PARAM_SIZE_HINT]: source.length,
        },
      }),
    ],
    ['.gz', gzipSync(source, { level: constants.Z_BEST_COMPRESSION })],
  ]) {
    // A copy that saves nothing would only be a second file to read.
    if (encoded.length >= source.length * 0.9) continue;
    await writeFile(path + extension, encoded);
    written++;
  }
}
console.log(`Precompressed ${written} files in ${root}`);
