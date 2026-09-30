import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { reporter } from './ci-report.mjs';

const directory = resolve(process.argv[2]);
const report = JSON.parse(readFileSync(join(directory, 'result.json'), 'utf8'));
if (!report.finished) {
  const finished = new Date().toISOString();
  const error = 'The local CI coordinator exited without finishing this run.';
  report.finished = finished;
  report.passed = false;
  report.error = error;
  for (const lane of report.lanes) {
    if (lane.finished) continue;
    lane.state = 'failed';
    lane.finished = finished;
    lane.exit_code = 1;
    lane.error = error;
    const path = lane.workspace && join(lane.workspace, '.local/ci-steps.json');
    if (path && existsSync(path)) {
      const steps = JSON.parse(readFileSync(path, 'utf8'));
      for (const step of steps) {
        if (step.finished) continue;
        step.finished = finished;
        step.passed = false;
        step.error = error;
      }
      writeFileSync(path, JSON.stringify(steps, null, 2) + '\n');
    }
  }
  const view = reporter(directory, report);
  await view.style(directory).catch(() => {});
  view.save();
}
