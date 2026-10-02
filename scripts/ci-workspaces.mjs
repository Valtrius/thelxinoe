import { execFileSync } from 'node:child_process';
import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  realpathSync,
  rmSync,
} from 'node:fs';
import { join, resolve, sep, toNamespacedPath } from 'node:path';

export function archiveWorkspace(report, lane) {
  if (!lane.workspace) return;
  const directory = realpathSync(report.directory);
  const expected = join(directory, 'workspaces', lane.phase);
  const workspace = realpathSync(lane.workspace);
  if (
    workspace !== realpathSync(expected) ||
    !workspace.startsWith(directory + sep)
  )
    throw Error('Lane workspace escaped its run');
  const evidence = join(directory, 'evidence', lane.phase);
  mkdirSync(evidence, { recursive: true });
  for (const folder of ['test-results', 'playwright-report']) {
    const source = join(workspace, folder);
    if (existsSync(source))
      cpSync(source, join(evidence, folder), { recursive: true });
  }
  // Keep diagnostics from every fixture, without retaining databases, secrets,
  // downloads, nested source copies or dependency trees.
  const local = join(workspace, '.local');
  function collect(source, destination, depth = 0) {
    if (!existsSync(source) || depth > 2) return;
    for (const entry of readdirSync(source, { withFileTypes: true })) {
      const path = join(source, entry.name);
      if (
        entry.isFile() &&
        /(?:result\.json|ci-[\w-]+\.json|\.(?:png|zip|log|ts))$/.test(
          entry.name,
        )
      ) {
        mkdirSync(destination, { recursive: true });
        cpSync(path, join(destination, entry.name));
      } else if (
        entry.isDirectory() &&
        (depth > 0 ||
          entry.name.startsWith('thelxinoe-') ||
          entry.name === 'player-ui' ||
          entry.name === 'server-verification')
      ) {
        collect(path, join(destination, entry.name), depth + 1);
      }
    }
  }
  collect(local, join(evidence, '.local'));
  const options = {
    cwd: resolve(report.origin.worktree),
    stdio: 'pipe',
    windowsHide: true,
  };
  try {
    execFileSync(
      'git',
      ['-c', 'core.longpaths=true', 'worktree', 'remove', '--force', workspace],
      options,
    );
  } catch (error) {
    if (
      process.platform !== 'win32' ||
      !/Filename too long|Result too large|Function not implemented|not a working tree/.test(
        String(error.stderr),
      )
    )
      throw error;
    // Git for Windows may unregister a worktree while failing to remove long
    // paths or Docker-created links. The exact run-owned path was checked above.
    rmSync(toNamespacedPath(workspace), {
      recursive: true,
      force: true,
      maxRetries: 5,
      retryDelay: 200,
    });
    execFileSync('git', ['worktree', 'prune'], options);
  }
  lane.workspace = null;
  lane.evidence = `evidence/${lane.phase}`;
}
