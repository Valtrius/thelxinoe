import { spawnSync } from 'node:child_process';

const executable = process.platform === 'win32' ? 'python' : 'python3';
const result = spawnSync(
  executable,
  ['-B', '-m', 'unittest', 'discover', '-s', 'tests', '-p', '*_test.py'],
  { stdio: 'inherit' },
);

if (result.error) throw result.error;
process.exit(result.status ?? 1);
