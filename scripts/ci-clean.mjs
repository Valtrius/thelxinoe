import { execFileSync } from 'node:child_process';
import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  realpathSync,
} from 'node:fs';
import { join, resolve, sep } from 'node:path';
import { acquireRun, jsonRead } from './ci-state.mjs';
import { cleanupResources } from './ci-resources.mjs';
import { reporter } from './ci-report.mjs';

if (!process.argv[2])
  throw Error('Usage: npm run ci:clean -- <finished-run-directory>');
const repository = realpathSync(resolve(import.meta.dirname, '..'));
const directory = realpathSync(resolve(process.argv[2]));
const report = jsonRead(join(directory, 'result.json'));
if (
  !report.finished ||
  !report.origin ||
  realpathSync(report.origin.worktree) !== repository ||
  directory !== realpathSync(report.directory)
)
  throw Error(
    'Choose a finished CI run belonging to this originating worktree',
  );
const release = await acquireRun(report.origin, {
  id: report.id,
  directory,
  pid: process.pid,
});
try {
  const errors = cleanupResources(report.resources, report.id, {
    removeImages: true,
  });
  if (errors.length) throw Error(errors.join('\n'));
  for (const lane of report.lanes) {
    if (!lane.workspace) continue;
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
      const path = join(workspace, folder);
      if (existsSync(path))
        cpSync(path, join(evidence, folder), { recursive: true });
    }
    const local = join(workspace, '.local');
    if (existsSync(local)) {
      for (const entry of readdirSync(local, { withFileTypes: true })) {
        if (
          entry.isFile() &&
          /(?:-result\.json|ci-images\.json|ci-steps\.json|\.png|\.zip)$/.test(
            entry.name,
          )
        )
          cpSync(join(local, entry.name), join(evidence, entry.name));
        if (
          entry.isDirectory() &&
          entry.name.startsWith('thelxinoe-recyclarr-')
        ) {
          const destination = join(evidence, entry.name);
          mkdirSync(destination, { recursive: true });
          for (const name of readdirSync(join(local, entry.name)))
            if (
              /^(?:result\.json|trace\.zip|services\.log|failure\.png|command-\d+\.log)$/.test(
                name,
              )
            )
              cpSync(join(local, entry.name, name), join(destination, name));
        }
      }
    }
    execFileSync('git', ['worktree', 'remove', '--force', workspace], {
      cwd: repository,
      stdio: 'inherit',
    });
    lane.workspace = null;
    lane.evidence = `evidence/${lane.phase}`;
    reporter(directory, report).save();
  }
  report.cleaned = new Date().toISOString();
  reporter(directory, report).save();
  console.log(
    `Removed finished lane worktrees; source snapshot and evidence retained: ${directory}`,
  );
} finally {
  await release();
}
