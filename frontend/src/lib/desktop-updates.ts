import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { writable } from 'svelte/store';
import { desktop } from './api';
import { publishDesktopActivity } from './activity';

export type DesktopUpdateStatus = {
  installed: string;
  policy: 'notify' | 'automatic';
  phase:
    'idle' | 'checking' | 'downloading' | 'ready' | 'waiting' | 'installing';
  release: { version: string; notes: string; bytes: number } | null;
  checked_at: number | null;
  error: string | null;
  received: number;
  total: number;
};
export const desktopUpdates = writable<DesktopUpdateStatus | null>(null);
export async function connectDesktopUpdates() {
  if (!desktop) return () => {};
  const stopActivity = desktopUpdates.subscribe((status) => {
    const phase = status?.phase;
    publishDesktopActivity(
      'update',
      status &&
        phase &&
        ['checking', 'downloading', 'waiting', 'installing'].includes(phase)
        ? [
            {
              id: 'desktop-update',
              target: 'connection',
              resource: null,
              state: phase === 'waiting' ? 'queued' : 'running',
              message:
                phase === 'waiting'
                  ? 'Desktop update waiting for idle'
                  : phase === 'checking'
                    ? 'Checking desktop updates'
                    : phase === 'downloading'
                      ? 'Downloading desktop update'
                      : 'Installing desktop update',
              media_ids: [],
            },
          ]
        : [],
    );
  });
  const stop = await listen<DesktopUpdateStatus>(
    'desktop-update-changed',
    (event) => desktopUpdates.set(event.payload),
  ).catch((error) => {
    stopActivity();
    publishDesktopActivity('update', []);
    throw error;
  });
  try {
    desktopUpdates.set(
      await invoke<DesktopUpdateStatus>('desktop_update_status'),
    );
  } catch {
    stop();
    stopActivity();
    publishDesktopActivity('update', []);
  }
  return () => {
    stop();
    stopActivity();
    publishDesktopActivity('update', []);
  };
}
export async function desktopUpdate(
  action: 'apply' | 'check' | 'download' | 'install' | 'policy',
  args = {},
) {
  desktopUpdates.set(
    await invoke<DesktopUpdateStatus>(`desktop_update_${action}`, args),
  );
}
