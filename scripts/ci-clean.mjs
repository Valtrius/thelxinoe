import { realpathSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { archiveWorkspace } from './ci-workspaces.mjs';
import { acquireRun, jsonRead } from './ci-state.mjs';
import { cleanupResources } from './ci-resources.mjs';
import { reporter } from './ci-report.mjs';

if (!process.argv[2])
  throw Error('Usage: pnpm run ci:clean <finished-run-directory>');
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
  if (!errors.length) {
    for (const lane of report.lanes) {
      try {
        archiveWorkspace(report, lane);
      } catch (error) {
        errors.push(`${lane.phase}: ${error.message}`);
      }
      reporter(directory, report).save();
    }
  }
  report.cleanup_errors = errors;
  if (!errors.length) report.cleaned = new Date().toISOString();
  reporter(directory, report).save();
  if (errors.length) throw Error(errors.join('\n'));
  console.log(
    `Removed finished lane worktrees; source snapshot and evidence retained: ${directory}`,
  );
} finally {
  await release();
}
