import type { User } from './api';
import type { ProviderAvailability } from './providers/availability';

export const sectionNames = [
  'Home',
  'Discover',
  'Movies',
  'Shows',
  'Music',
  'Playlists',
  'YouTube',
  'Twitch',
  'Kick',
  'Statistics',
  'Settings',
] as const;
export type Section = (typeof sectionNames)[number];
export const personalSettings = [
  'account',
  'playback',
  'online',
  'devices',
] as const;
export const adminSettings = [
  'server',
  'analysis',
  'providers',
  'services',
  'retention',
  'backups',
  'people',
  'jobs',
  'audit',
] as const;
export const desktopSettings = ['mpv', 'connection'] as const;
export type SettingsSection =
  | (typeof personalSettings)[number]
  | (typeof adminSettings)[number]
  | (typeof desktopSettings)[number];
export type AppRoute = {
  section: Section;
  settings: SettingsSection;
  detail?: { kind: 'movie' | 'tv'; id: number };
  requests?: boolean;
  item?: string;
  ancestors?: string[];
  playlist?: string;
  service?: string;
  workflow?: 'setup' | 'updates';
  watchlist?: number;
  query?: string;
  collection?: string;
  range?: string;
  platform?: string;
  scope?: string;
};
export const homeRoute: AppRoute = { section: 'Home', settings: 'account' };
export function sectionRoute(
  name: string,
  settings: string = 'account',
): AppRoute {
  const section = name === 'History' ? 'Statistics' : name;
  const normalized =
    settings === 'library'
      ? 'services'
      : settings === 'server-updates'
        ? 'server'
        : settings;
  return {
    section: sectionNames.includes(section as Section)
      ? (section as Section)
      : 'Home',
    settings: [
      ...personalSettings,
      ...adminSettings,
      ...desktopSettings,
    ].includes(normalized as SettingsSection)
      ? (normalized as SettingsSection)
      : 'account',
  };
}
export function readRoute(url: URL, fallback: AppRoute = homeRoute): AppRoute {
  const [path, search = ''] = url.hash.slice(1).split('?');
  const parts = path.split('/').map((part) => {
    try {
      return decodeURIComponent(part);
    } catch {
      return '';
    }
  });
  const query = new URLSearchParams(search);
  const base = { ...homeRoute, settings: fallback.settings };
  if (parts[0] === 'home') return base;
  if (parts[0] === 'discover') {
    const detail =
      ['movie', 'tv'].includes(parts[1]) && /^\d+$/.test(parts[2])
        ? { kind: parts[1] as 'movie' | 'tv', id: Number(parts[2]) }
        : undefined;
    return {
      ...base,
      section: 'Discover',
      detail,
      query: query.get('q') ?? undefined,
    };
  }
  if (parts[0] === 'requests')
    return { ...base, section: 'Discover', requests: true };
  if (parts[0] === 'library') {
    const section = (
      { movies: 'Movies', shows: 'Shows', music: 'Music' } as const
    )[parts[1] as 'movies' | 'shows' | 'music'];
    return section
      ? {
          ...base,
          section,
          item:
            parts.at(-1) !== parts[1] ? parts.at(-1) || undefined : undefined,
          ancestors: parts.slice(2, -1).filter(Boolean),
          query: query.get('q') ?? undefined,
          collection: query.get('collection') ?? undefined,
        }
      : base;
  }
  if (parts[0] === 'playlists')
    return { ...base, section: 'Playlists', playlist: parts[1] || undefined };
  if (parts[0] === 'online') {
    const section = (
      { youtube: 'YouTube', twitch: 'Twitch', kick: 'Kick' } as const
    )[parts[1] as 'youtube' | 'twitch' | 'kick'];
    return section
      ? {
          ...base,
          section,
          watchlist:
            parts[2] === 'watchlists' && /^\d+$/.test(parts[3])
              ? Number(parts[3])
              : undefined,
        }
      : base;
  }
  if (parts[0] === 'statistics')
    return {
      ...base,
      section: 'Statistics',
      range: query.get('range') ?? undefined,
      platform: query.get('platform') ?? undefined,
      scope: query.get('scope') ?? undefined,
    };
  if (parts[0] === 'settings' || parts[0] === 'admin')
    return {
      ...sectionRoute('Settings', parts[1]),
      service:
        parts[1] === 'services' &&
        [
          'seerr',
          'recyclarr',
          'radarr',
          'sonarr',
          'lidarr',
          'bazarr',
          'prowlarr',
          'nzbget',
        ].includes(parts[2])
          ? parts[2]
          : undefined,
      workflow:
        parts[3] === 'setup' || parts[3] === 'updates' ? parts[3] : undefined,
    };
  if (url.hash) return base;
  if (url.searchParams.has('youtube_link')) return sectionRoute('YouTube');
  const legacy = url.searchParams.get('section');
  return legacy ? sectionRoute(legacy, fallback.settings) : fallback;
}
export function allowedRoute(
  route: AppRoute,
  user: User,
  desktop: boolean,
  providers: ProviderAvailability,
): AppRoute {
  const platform = route.section.toLowerCase();
  if (
    Object.hasOwn(providers, platform) &&
    !providers[platform as keyof ProviderAvailability]
  )
    return { ...homeRoute };
  if (route.section !== 'Settings') return route;
  if (
    (adminSettings as readonly string[]).includes(route.settings) &&
    user.role !== 'admin'
  )
    return sectionRoute('Settings');
  if (
    (!desktop &&
      (desktopSettings as readonly string[]).includes(route.settings)) ||
    (route.settings === 'online' && !Object.values(providers).some(Boolean))
  )
    return sectionRoute('Settings');
  return route;
}
export function routeHref(route: AppRoute): string {
  let path: string;
  const query = new URLSearchParams();
  switch (route.section) {
    case 'Home':
      path = 'home';
      break;
    case 'Discover':
      path = route.detail
        ? `discover/${route.detail.kind}/${route.detail.id}`
        : route.requests
          ? 'requests'
          : 'discover';
      if (route.query) query.set('q', route.query);
      break;
    case 'Movies':
    case 'Shows':
    case 'Music':
      path = `library/${route.section.toLowerCase()}${route.item ? '/' + [...(route.ancestors ?? []), route.item].map(encodeURIComponent).join('/') : ''}`;
      if (route.query) query.set('q', route.query);
      if (route.collection) query.set('collection', route.collection);
      break;
    case 'Playlists':
      path = `playlists${route.playlist ? '/' + encodeURIComponent(route.playlist) : ''}`;
      break;
    case 'YouTube':
    case 'Twitch':
    case 'Kick':
      path = `online/${route.section.toLowerCase()}${route.watchlist ? '/watchlists/' + route.watchlist : ''}`;
      break;
    case 'Statistics':
      path = 'statistics';
      for (const key of ['range', 'platform', 'scope'] as const)
        if (route[key]) query.set(key, route[key]!);
      break;
    case 'Settings':
      path = `${(adminSettings as readonly string[]).includes(route.settings) ? 'admin' : 'settings'}/${route.settings}${route.service ? '/' + encodeURIComponent(route.service) : ''}${route.workflow ? '/' + route.workflow : ''}`;
      break;
  }
  return `#${path}${query.size ? '?' + query : ''}`;
}
export function followLink(event: MouseEvent): boolean {
  return (
    !event.defaultPrevented &&
    event.button === 0 &&
    !event.ctrlKey &&
    !event.metaKey &&
    !event.shiftKey &&
    !event.altKey
  );
}
