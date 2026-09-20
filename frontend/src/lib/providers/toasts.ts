import { writable } from 'svelte/store';

export type ToastTone = 'info' | 'success' | 'warning' | 'error';

export interface ToastInput {
  key: string;
  tone?: ToastTone;
  title: string;
  message: string;
  detail?: string | null;
  actionLabel?: string | null;
  onAction?: (() => void) | null;
  durationMs?: number | null;
  busy?: boolean;
  progress?: { value?: number; max: number; label: string };
}

export interface Toast extends ToastInput {
  id: string;
  tone: ToastTone;
  durationMs: number | null;
}

const timers = new Map<string, ReturnType<typeof setTimeout>>();
let nextId = 0;

export const toasts = writable<Toast[]>([]);

function clearTimer(id: string) {
  const timer = timers.get(id);
  if (timer) clearTimeout(timer);
  timers.delete(id);
}

function scheduleDismissal(toast: Toast) {
  clearTimer(toast.id);
  if (toast.durationMs === null) return;
  timers.set(
    toast.id,
    setTimeout(() => dismissToast(toast.id), toast.durationMs),
  );
}

export function showToast(input: ToastInput): string {
  let toastId = '';
  let nextToast: Toast | null = null;
  toasts.update((current) => {
    const existing = current.find((toast) => toast.key === input.key);
    toastId = existing?.id ?? `toast-${++nextId}`;
    nextToast = {
      ...input,
      id: toastId,
      tone: input.tone ?? 'info',
      durationMs: input.durationMs === undefined ? 6_000 : input.durationMs,
    };
    return existing
      ? current.map((toast) => (toast.id === toastId ? nextToast! : toast))
      : [...current, nextToast];
  });
  scheduleDismissal(nextToast!);
  return toastId;
}

export function dismissToast(id: string) {
  clearTimer(id);
  toasts.update((current) => current.filter((toast) => toast.id !== id));
}

export function dismissToastByKey(key: string) {
  toasts.update((current) => {
    const removed = current.filter((toast) => toast.key === key);
    for (const toast of removed) clearTimer(toast.id);
    return current.filter((toast) => toast.key !== key);
  });
}

export function clearToasts() {
  for (const timer of timers.values()) clearTimeout(timer);
  timers.clear();
  toasts.set([]);
}
