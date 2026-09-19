import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
const exe = process.platform === 'win32' ? 'npx.cmd' : 'npx';
if (!existsSync('apps/desktop/icons/icon.ico')) {
  const icon = spawnSync(
    exe,
    [
      'tauri',
      'icon',
      'frontend/public/icon.svg',
      '--output',
      'apps/desktop/icons',
    ],
    { stdio: 'inherit', shell: process.platform === 'win32' },
  );
  if (icon.status) process.exit(icon.status);
}
const result = spawnSync(
  exe,
  ['tauri', process.argv.includes('--dev') ? 'dev' : 'build'],
  {
    cwd: 'apps/desktop',
    stdio: 'inherit',
    shell: process.platform === 'win32',
  },
);
process.exit(result.status ?? 1);
