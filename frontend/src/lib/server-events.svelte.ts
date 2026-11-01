import type { ServerEvent } from './api';
import { navigationCache } from './navigation-cache';

// Pages reload what a revision covers each time it moves.
export const revisions = $state({
  media: 0,
  catalog: 0,
  account: 0,
  preferences: 0,
  settings: 0,
});
type Revision = keyof typeof revisions;
type CachedPage = 'catalog' | 'discover' | 'youtube' | 'twitch' | 'kick';

const everyPage: CachedPage[] = [
  'catalog',
  'discover',
  'youtube',
  'twitch',
  'kick',
];
const effects: Record<string, { reload?: Revision[]; stale?: CachedPage[] }> = {
  'server.reconnected': {
    reload: ['media', 'catalog', 'account', 'settings'],
    stale: everyPage,
  },
  'account.profile.changed': { reload: ['preferences'], stale: everyPage },
  'catalog.changed': { reload: ['media', 'catalog'], stale: ['catalog'] },
  'catalog.status.changed': {
    reload: ['catalog'],
    stale: ['catalog', 'discover'],
  },
  'media-state.changed': {
    reload: ['media', 'catalog'],
    stale: ['catalog', 'youtube'],
  },
  'playback.changed': {
    reload: ['media', 'catalog'],
    stale: ['catalog', 'youtube'],
  },
  'playlists.changed': { reload: ['media'], stale: ['catalog', 'youtube'] },
  'youtube.changed': { reload: ['media'], stale: ['youtube'] },
  'online.download.changed': {
    reload: ['media', 'settings'],
    stale: ['youtube'],
  },
  'online.account.changed': {
    reload: ['media', 'account'],
    stale: ['youtube', 'twitch', 'kick'],
  },
  'online.configuration.changed': {
    reload: ['media', 'account'],
    stale: ['youtube', 'twitch', 'kick'],
  },
  'preferences.changed': { reload: ['preferences'] },
  'server.settings.changed': { reload: ['preferences', 'settings'] },
  'stack.changed': { reload: ['settings'], stale: ['discover'] },
  'jobs.changed': { reload: ['settings'] },
  'recyclarr.changed': { reload: ['settings'] },
  'retention.changed': { reload: ['settings'] },
  'users.changed': { reload: ['settings'] },
  'requests.changed': { stale: ['discover'] },
  'attention.changed': { stale: ['discover'] },
};

type Listener = (event: ServerEvent) => void;
let listeners: Listener[] = [];

export function onServerEvent(listener: Listener) {
  listeners = [...listeners, listener];
  return () => {
    listeners = listeners.filter((other) => other !== listener);
  };
}

// `progress` marks a playback report that repeats its session's state: it only
// moves a position, which pages may show a moment late.
export function publish(event: ServerEvent, progress = false) {
  if (progress) navigationCache.expire('catalog', 'youtube');
  else {
    const effect = effects[event.kind];
    if (effect?.stale) navigationCache.invalidate(...effect.stale);
    for (const revision of effect?.reload ?? []) revisions[revision]++;
  }
  for (const listener of listeners) listener(event);
}
