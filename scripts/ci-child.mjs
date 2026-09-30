import { spawnSync } from 'node:child_process';

// The run ID in this wrapper's command line lets recovery validate PID ownership.
const [, cwd, command, ...args] = process.argv.slice(2);
const result = spawnSync(command, args, {
  cwd,
  env: process.env,
  stdio: 'inherit',
  windowsHide: true,
});
if (result.error) console.error(result.error.message);
process.exitCode = result.error ? 1 : (result.status ?? 1);
