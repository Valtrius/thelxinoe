import { get, writable } from 'svelte/store';
import { api } from './api';

export type AttentionTarget =
  | 'services'
  | 'server'
  | 'jobs'
  | 'online'
  | 'requests'
  | 'movies'
  | 'shows'
  | 'music';
export type AttentionItem = {
  id: string;
  severity: 'info' | 'warning' | 'error';
  target: AttentionTarget;
  resource: string | null;
  message: string;
  revision: string;
  dismissible: boolean;
  media_ids: string[];
};
export type RequestAttention = Pick<AttentionItem, 'id' | 'revision'>;
export const attention = writable<AttentionItem[]>([]);
export const attentionErrors = writable<Record<string, string>>({});
const pending = new Map<string, Promise<boolean>>();
let owner = '';
let epoch = 0;
let request = 0;
let refreshing = false;
let queued = false;

export function attentionDescription(items: AttentionItem[]) {
  return items
    .map(
      (item) =>
        `${item.severity === 'error' ? 'Issue' : item.severity === 'warning' ? 'Warning' : 'Info'}: ${item.message}`,
    )
    .join('\n');
}

export async function refreshAttention(): Promise<void> {
  if (!owner) return;
  if (refreshing) {
    queued = true;
    return;
  }
  const session = epoch;
  const current = ++request;
  refreshing = true;
  try {
    const result = await api<{ items: AttentionItem[] }>('/me/attention');
    if (
      session === epoch &&
      current === request &&
      JSON.stringify(get(attention)) !== JSON.stringify(result.items)
    )
      attention.set(result.items);
  } catch {
    // Retain the last known conditions until reconnection can verify them.
  } finally {
    if (session === epoch) {
      refreshing = false;
      if (queued) {
        queued = false;
        void refreshAttention();
      }
    }
  }
}

export function connectAttention(userId: string) {
  owner = userId;
  epoch++;
  request++;
  refreshing = false;
  queued = false;
  attention.set([]);
  attentionErrors.set({});
  pending.clear();
  const refresh = () => {
    if (!document.hidden) void refreshAttention();
  };
  window.addEventListener('focus', refresh);
  window.addEventListener('thelxinoe-attention-refresh', refresh);
  document.addEventListener('visibilitychange', refresh);
  void refreshAttention();
  return () => {
    owner = '';
    epoch++;
    request++;
    attention.set([]);
    attentionErrors.set({});
    pending.clear();
    window.removeEventListener('focus', refresh);
    window.removeEventListener('thelxinoe-attention-refresh', refresh);
    document.removeEventListener('visibilitychange', refresh);
  };
}

export async function acknowledgeAttention(
  item: AttentionItem,
): Promise<boolean> {
  if (!owner || !item.dismissible) return false;
  const key = item.id + ':' + item.revision;
  if (pending.has(key)) return pending.get(key)!;
  const session = epoch;
  const action = (async () => {
    try {
      await api(`/me/attention/${encodeURIComponent(item.id)}`, 'PUT', {
        revision: item.revision,
      });
      if (session !== epoch) return false;
      request++;
      attention.update((items) =>
        items.filter(
          (current) =>
            current.id !== item.id || current.revision !== item.revision,
        ),
      );
      attentionErrors.update((errors) => {
        const next = { ...errors };
        delete next[item.id];
        return next;
      });
      return true;
    } catch {
      if (session === epoch) {
        attentionErrors.update((errors) => ({
          ...errors,
          [item.id]: 'Could not clear this marker. Hover again to retry.',
        }));
        void refreshAttention();
      }
      return false;
    } finally {
      if (session === epoch) pending.delete(key);
    }
  })();
  pending.set(key, action);
  return action;
}
