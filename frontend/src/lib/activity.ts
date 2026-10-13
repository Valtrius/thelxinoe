import { derived, get, writable } from 'svelte/store';
import type { AttentionTarget } from './attention';

export type ActivityItem = {
  id: string;
  target: AttentionTarget | 'connection' | 'mpv' | 'analysis';
  resource: string | null;
  state: 'running' | 'queued' | 'unknown';
  message: string;
  media_ids: string[];
};

export const serverActivity = writable<ActivityItem[]>([]);
export const desktopActivity = writable<ActivityItem[]>([]);
const pendingActivity = writable<
  (ActivityItem & { source: 'server' | 'desktop' })[]
>([]);
const desktopSources = new Map<'update' | 'tools', ActivityItem[]>();
const connected = writable(true);
export const activity = derived(
  [serverActivity, desktopActivity, pendingActivity, connected],
  ([server, desktop, pending, online]) => [
    ...[...server, ...pending.filter((item) => item.source === 'server')].map(
      (item) => (online ? item : { ...item, state: 'unknown' as const }),
    ),
    ...desktop,
    ...pending.filter((item) => item.source === 'desktop'),
  ],
);
let sequence = 0;
let epoch = 0;

export function activityDescription(items: ActivityItem[]) {
  return [
    ...new Set(
      items.map(
        (item) =>
          `${item.state === 'unknown' ? 'Status unknown' : item.state === 'queued' ? 'Waiting' : 'In progress'}: ${item.message}`,
      ),
    ),
  ].join('\n');
}

export function setActivityConnected(value: boolean) {
  connected.set(value);
}

export function resetActivity() {
  epoch++;
  serverActivity.set([]);
  pendingActivity.set([]);
  connected.set(true);
}

export function publishDesktopActivity(
  source: 'update' | 'tools',
  items: ActivityItem[],
) {
  desktopSources.set(source, items);
  desktopActivity.set([...desktopSources.values()].flat());
}

// Request work belongs to the session, so navigation does not end its lifetime.
export function startActivity(
  item: Omit<ActivityItem, 'id' | 'state' | 'media_ids'> & {
    media_ids?: string[];
  },
  source: 'server' | 'desktop' = 'server',
) {
  const id = `request:${++sequence}`;
  const session = epoch;
  const timer = setTimeout(() => {
    if (session === epoch)
      pendingActivity.update((items) => [
        ...items,
        {
          ...item,
          id,
          source,
          state: 'running',
          media_ids: item.media_ids ?? [],
        },
      ]);
  }, 400);
  return () => {
    clearTimeout(timer);
    if (session === epoch)
      pendingActivity.update((items) => items.filter((item) => item.id !== id));
  };
}

export function hasServerActivity() {
  return (
    get(serverActivity).length > 0 ||
    get(pendingActivity).some((item) => item.source === 'server')
  );
}
