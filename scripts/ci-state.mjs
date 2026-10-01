import { execFileSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { createConnection, createServer } from 'node:net';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmdirSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

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

export function originIdentity(repository) {
  const worktree = realpathSync(repository);
  const git = (...args) =>
    execFileSync('git', args, {
      cwd: worktree,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim();
  let branch = null;
  try {
    branch = git('symbolic-ref', '--quiet', '--short', 'HEAD');
  } catch {
    /* Detached HEAD. */
  }
  return {
    worktree,
    key: createHash('sha256')
      .update(process.platform === 'win32' ? worktree.toLowerCase() : worktree)
      .digest('hex')
      .slice(0, 24),
    branch,
    revision: git('rev-parse', 'HEAD'),
    dirty: !!git('status', '--porcelain', '--untracked-files=all'),
  };
}

export const originLabel = (origin) => origin?.branch ?? 'Detached HEAD';
function address(origin) {
  const name = `thelxinoe-ci-${origin.key}`;
  return process.platform === 'win32'
    ? `\\\\.\\pipe\\${name}`
    : join(tmpdir(), `${name}.sock`);
}

export function requestRun(origin, action = 'status') {
  return new Promise((resolve, reject) => {
    const socket = createConnection(address(origin));
    let contents = '';
    socket.setTimeout(5000, () =>
      socket.destroy(Error('CI coordinator did not respond')),
    );
    socket.once('error', reject);
    socket.once('connect', () =>
      socket.write(JSON.stringify({ action }) + '\n'),
    );
    socket.on('data', (bytes) => {
      contents += bytes;
    });
    socket.once('end', () => {
      try {
        resolve(JSON.parse(contents));
      } catch (error) {
        reject(error);
      }
    });
  });
}

export async function acquireRun(origin, context, stop) {
  const metadata = join(origin.worktree, '.local/ci/active.json');
  mkdirSync(join(origin.worktree, '.local/ci'), { recursive: true });
  const server = createServer((socket) => {
    socket.setTimeout(5000, () => socket.destroy());
    socket.once('data', (bytes) => {
      let action;
      try {
        action = JSON.parse(String(bytes)).action;
      } catch {
        action = 'status';
      }
      socket.end(JSON.stringify(context), () => {
        if (action === 'stop' && stop) stop();
      });
    });
  });
  const listen = () =>
    new Promise((resolve, reject) => {
      server.once('error', reject);
      server.listen(address(origin), () => {
        server.removeListener('error', reject);
        resolve();
      });
    });
  try {
    await listen();
  } catch (error) {
    if (error.code !== 'EADDRINUSE') throw error;
    let active;
    try {
      active = await requestRun(origin);
    } catch (probe) {
      if (process.platform === 'win32' || probe.code !== 'ECONNREFUSED')
        throw error;
      const old = existsSync(metadata) && jsonRead(metadata);
      if (!old || alive(old.pid)) throw error;
      // Serialize stale Unix socket recovery; never unlink a live owner's socket.
      const recovery = address(origin) + '.recovery';
      mkdirSync(recovery);
      try {
        try {
          active = await requestRun(origin);
        } catch (retry) {
          if (retry.code !== 'ECONNREFUSED') throw retry;
          unlinkSync(address(origin));
          await listen();
        }
      } finally {
        rmdirSync(recovery);
      }
    }
    if (active)
      throw Error(
        `Another local CI run owns this worktree. Report: ${join(active.directory, 'index.html')}`,
        { cause: error },
      );
  }
  try {
    if (existsSync(metadata)) {
      const previous = jsonRead(metadata);
      if (previous.id !== context.id) {
        const result = join(previous.directory, 'result.json');
        if (!existsSync(result) || !jsonRead(result).finished)
          throw Error(
            `An interrupted run still owns this worktree. Recover it first: node scripts/ci-local.mjs --recover "${previous.directory}"`,
          );
      }
    }
    jsonWrite(metadata, context);
  } catch (error) {
    await new Promise((resolve) => server.close(resolve));
    throw error;
  }
  return async () => {
    if (existsSync(metadata) && jsonRead(metadata).id === context.id)
      unlinkSync(metadata);
    await new Promise((resolve) => server.close(resolve));
  };
}
