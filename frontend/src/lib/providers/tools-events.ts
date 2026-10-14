import { listen } from '@tauri-apps/api/event';
import { toolsState } from './tools-state';
import type { ToolOperation } from './tools-api';
import { toolNames } from './tools-api';
import { publishDesktopActivity } from '../activity';
export async function connectTools() {
  const stopActivity = toolsState.subscribe((state) => {
    const operation = state.snapshot?.operation;
    publishDesktopActivity(
      'tools',
      operation?.tool && ['downloading', 'verifying'].includes(operation.phase)
        ? [
            {
              id: `desktop-tool:${operation.tool}`,
              target: 'mpv',
              resource: operation.tool,
              state: state.loadError ? 'unknown' : 'running',
              message: `${operation.phase === 'downloading' ? 'Downloading' : 'Verifying'} ${toolNames[operation.tool]}`,
              media_ids: [],
            },
          ]
        : [],
    );
  });
  const unlisten = await Promise.all([
    listen<ToolOperation>('tools-progress', ({ payload }) =>
      toolsState.progress(payload),
    ),
    listen('tools-changed', () => void toolsState.refresh().catch(() => {})),
  ]).catch((error) => {
    stopActivity();
    publishDesktopActivity('tools', []);
    throw error;
  });
  void toolsState.refresh().catch(() => {});
  return () => {
    unlisten.forEach((stop) => stop());
    stopActivity();
    publishDesktopActivity('tools', []);
  };
}
