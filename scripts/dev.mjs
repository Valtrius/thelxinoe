import { spawn, spawnSync } from 'node:child_process';

const isWindows = process.platform === 'win32';
const children = [];
let stopping = false;

function commandForSpawn(command, args) {
  if (isWindows && command.toLowerCase().endsWith('.cmd')) {
    return {
      command: process.env.ComSpec ?? 'cmd.exe',
      args: ['/d', '/s', '/c', [command, ...args].join(' ')],
    };
  }
  return { command, args };
}

function stopChild(child) {
  if (!child.pid || child.exitCode !== null) return;
  if (isWindows) {
    spawnSync('taskkill', ['/pid', String(child.pid), '/t', '/f'], {
      stdio: 'ignore',
    });
  } else {
    child.kill('SIGTERM');
  }
}

function stop(code) {
  if (stopping) return;
  stopping = true;
  for (const child of children) stopChild(child);
  process.exitCode = code;
}

function start(name, command, args) {
  const invocation = commandForSpawn(command, args);
  const child = spawn(invocation.command, invocation.args, {
    stdio: 'inherit',
  });
  children.push(child);
  child.once('error', (error) => {
    console.error(`${name} failed to start: ${error.message}`);
    stop(1);
  });
  child.once('exit', (code, signal) => {
    if (stopping) return;
    if (signal) console.error(`${name} stopped by ${signal}.`);
    stop(code ?? 1);
  });
}

process.once('SIGINT', () => stop(130));
process.once('SIGTERM', () => stop(143));

start('server', 'cargo', ['run', '--locked', '-p', 'thelxinoe-server']);
start('web', isWindows ? 'npm.cmd' : 'npm', ['run', 'dev:web']);
