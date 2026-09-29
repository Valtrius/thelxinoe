import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { writable } from 'svelte/store';
import { desktop } from './api';

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
  const stop = await listen<DesktopUpdateStatus>(
    'desktop-update-changed',
    (event) => desktopUpdates.set(event.payload),
  );
  try {
    desktopUpdates.set(
      await invoke<DesktopUpdateStatus>('desktop_update_status'),
    );
  } catch {
    stop();
  }
  return stop;
}
export async function desktopUpdate(
  action: 'apply' | 'check' | 'download' | 'install' | 'policy',
  args = {},
) {
  desktopUpdates.set(
    await invoke<DesktopUpdateStatus>(`desktop_update_${action}`, args),
  );
}
