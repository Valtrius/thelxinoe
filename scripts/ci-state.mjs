import { randomUUID } from 'node:crypto';
import {
  existsSync,
  readFileSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';

export function atomicWrite(path, contents) {
  const temporary = `${path}.${process.pid}-${randomUUID()}.tmp`;
  try {
    writeFileSync(temporary, contents);
    const deadline = Date.now() + 1000;
    const pause = new Int32Array(new SharedArrayBuffer(4));
    for (;;) {
      try {
        renameSync(temporary, path);
        break;
      } catch (error) {
        // Windows readers can briefly deny replacement of the live report.
        if (
          process.platform !== 'win32' ||
          !['EPERM', 'EACCES', 'EBUSY'].includes(error.code) ||
          Date.now() >= deadline
        )
          throw error;
        Atomics.wait(pause, 0, 0, 50);
      }
    }
  } finally {
    if (existsSync(temporary)) unlinkSync(temporary);
  }
}

export const jsonWrite = (path, value) =>
  atomicWrite(path, JSON.stringify(value, null, 2) + '\n');
export const jsonRead = (path) => JSON.parse(readFileSync(path, 'utf8'));
export const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code === 'EPERM';
  }
};
