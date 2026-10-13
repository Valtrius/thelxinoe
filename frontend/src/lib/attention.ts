import { get, writable } from 'svelte/store';
import { api } from './api';
import {
  activityDescription,
  hasServerActivity,
  resetActivity,
  serverActivity,
  setActivityConnected,
  type ActivityItem,
} from './activity';

export type AttentionTarget =
  | 'services'
  | 'server'
  | 'backups'
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
let refreshing: Promise<void> | null = null;
let queued = false;

export function attentionDescription(
  items: AttentionItem[],
  activity: ActivityItem[] = [],
) {
  return [
    activityDescription(activity),
    items
      .map(
        (item) =>
          `${item.severity === 'error' ? 'Issue' : item.severity === 'warning' ? 'Warning' : 'Info'}: ${item.message}`,
      )
      .join('\n'),
  ]
    .filter(Boolean)
    .join('\n');
}

export function refreshAttention(): Promise<void> {
  if (!owner) return Promise.resolve();
  if (refreshing) {
    queued = true;
    return refreshing;
  }
  const session = epoch;
  refreshing = (async () => {
    do {
      queued = false;
      const current = ++request;
      try {
        const result = await api<{
          items: AttentionItem[];
          activity?: ActivityItem[];
        }>('/me/attention');
        if (session === epoch && current === request) {
          if (JSON.stringify(get(attention)) !== JSON.stringify(result.items))
            attention.set(result.items);
          const next = result.activity ?? [];
          if (JSON.stringify(get(serverActivity)) !== JSON.stringify(next))
            serverActivity.set(next);
          setActivityConnected(true);
        }
      } catch {
        // Retain conditions, but stop claiming unverified activity is running.
        if (session === epoch) setActivityConnected(false);
      }
    } while (session === epoch && queued);
  })().finally(() => {
    if (session === epoch) refreshing = null;
  });
  return refreshing;
}

export function connectAttention(userId: string) {
  owner = userId;
  epoch++;
  request++;
  refreshing = null;
  queued = false;
  attention.set([]);
  attentionErrors.set({});
  resetActivity();
  pending.clear();
  const refresh = () => {
    if (!document.hidden) void refreshAttention();
  };
  window.addEventListener('focus', refresh);
  window.addEventListener('thelxinoe-attention-refresh', refresh);
  document.addEventListener('visibilitychange', refresh);
  const timer = setInterval(() => {
    if (hasServerActivity()) refresh();
  }, 5000);
  void refreshAttention();
  return () => {
    owner = '';
    epoch++;
    request++;
    attention.set([]);
    attentionErrors.set({});
    resetActivity();
    clearInterval(timer);
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
