import { spawnSync } from 'node:child_process';
const exe = process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm';
const icon = spawnSync(
  exe,
  [
    'exec',
    'tauri',
    'icon',
    'frontend/public/icon.svg',
    '--output',
    'apps/desktop/icons',
  ],
  { stdio: 'inherit', shell: process.platform === 'win32' },
);
if (icon.status !== 0) process.exit(icon.status ?? 1);
const result = spawnSync(
  exe,
  ['exec', 'tauri', process.argv.includes('--dev') ? 'dev' : 'build'],
  {
    cwd: 'apps/desktop',
    stdio: 'inherit',
    shell: process.platform === 'win32',
  },
);
process.exit(result.status ?? 1);
