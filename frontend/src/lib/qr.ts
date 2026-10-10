import { encode } from 'uqr';

/** SVG path for the dark modules, merging each row's adjacent modules into one rectangle. */
export function modulePath(modules: readonly (readonly boolean[])[]) {
  let path = '';
  modules.forEach((row, y) => {
    for (let x = 0; x < row.length; x++) {
      if (!row[x]) continue;
      const start = x;
      while (row[x + 1]) x++;
      const width = x - start + 1;
      path += `M${start} ${y}h${width}v1h-${width}z`;
    }
  });
  return path;
}

/** Encodes `value` with the four-module quiet zone scanners need around the symbol. */
export function qrCode(value: string) {
  const { size, data } = encode(value, { ecc: 'M', border: 4 });
  return { size, path: modulePath(data) };
}
