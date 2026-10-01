import { execFileSync } from 'node:child_process';
import { alive } from './ci-state.mjs';

export function stopProcess(pid, identity) {
  if (!Number.isSafeInteger(pid) || pid <= 0 || !alive(pid)) return;
  let command;
  try {
    command =
      process.platform === 'win32'
        ? execFileSync(
            'powershell.exe',
            [
              '-NoProfile',
              '-Command',
              `(Get-CimInstance Win32_Process -Filter "ProcessId = ${pid}").CommandLine`,
            ],
            { encoding: 'utf8', windowsHide: true, timeout: 30000 },
          )
        : execFileSync('ps', ['-p', String(pid), '-o', 'args='], {
            encoding: 'utf8',
            timeout: 30000,
          });
  } catch (error) {
    if (!alive(pid)) return;
    throw error;
  }
  if (
    !identity.every((part) =>
      command
        .toLowerCase()
        .replaceAll('\\', '/')
        .includes(part.toLowerCase().replaceAll('\\', '/')),
    )
  )
    throw Error(`Refusing to stop PID ${pid}: process ownership changed`);
  try {
    if (process.platform === 'win32')
      execFileSync('taskkill.exe', ['/PID', String(pid), '/T', '/F'], {
        windowsHide: true,
        stdio: 'pipe',
        timeout: 30000,
      });
    else {
      try {
        process.kill(-pid, 'SIGKILL');
      } catch (error) {
        if (error.code !== 'ESRCH') throw error;
        if (alive(pid)) process.kill(pid, 'SIGKILL');
      }
    }
  } catch (error) {
    if (alive(pid)) throw error;
  }
}
