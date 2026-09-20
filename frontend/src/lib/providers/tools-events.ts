import { listen } from '@tauri-apps/api/event';
import { toolsState } from './tools-state';
import type { ToolOperation } from './tools-api';
export async function connectTools() {
  const unlisten = await Promise.all([
    listen<ToolOperation>('tools-progress', ({ payload }) =>
      toolsState.progress(payload),
    ),
    listen('tools-changed', () => void toolsState.refresh().catch(() => {})),
  ]);
  void toolsState.refresh().catch(() => {});
  return () => unlisten.forEach((stop) => stop());
}
