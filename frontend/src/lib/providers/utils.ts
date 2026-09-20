import { createSubscriber } from 'svelte/reactivity';
import type { Route } from './types';

export const routes: Route[] = [
  '/youtube',
  '/twitch',
  '/kick',
  '/statistics',
  '/settings',
];

export function isRoute(value: string): value is Route {
  return routes.includes(value as Route);
}

export function formatClock(totalSeconds?: number | null): string {
  if (
    totalSeconds === null ||
    totalSeconds === undefined ||
    !Number.isFinite(totalSeconds) ||
    totalSeconds < 0
  ) {
    return '—';
  }
  const seconds = Math.floor(totalSeconds);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  return hours > 0
    ? `${hours}:${minutes.toString().padStart(2, '0')}:${remainder.toString().padStart(2, '0')}`
    : `${minutes}:${remainder.toString().padStart(2, '0')}`;
}

export function relativeTime(value?: string | null, now = Date.now()): string {
  if (!value) return 'Never';
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return 'Unknown';
  const seconds = Math.round((timestamp - now) / 1000);
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' });
  if (Math.abs(seconds) < 60) return formatter.format(seconds, 'second');
  const minutes = Math.round(seconds / 60);
  if (Math.abs(minutes) < 60) return formatter.format(minutes, 'minute');
  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 24) return formatter.format(hours, 'hour');
  return formatter.format(Math.round(hours / 24), 'day');
}

export function elapsedTime(value: string, now = Date.now()): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return '—';
  return formatClock(Math.max(0, (now - timestamp) / 1000));
}

let liveNowMs = Date.now();
const subscribeLiveNow = createSubscriber((update) => {
  liveNowMs = Date.now();
  const interval = window.setInterval(() => {
    liveNowMs = Date.now();
    update();
  }, 1000);
  return () => window.clearInterval(interval);
});

/** Wall-clock ms that ticks once per second while observed in a reactive context. */
export function liveNow(): number {
  subscribeLiveNow();
  return liveNowMs;
}

export function formatViewers(viewers: number): string {
  return new Intl.NumberFormat(undefined, {
    notation: viewers >= 1000 ? 'compact' : 'standard',
    maximumFractionDigits: 1,
  }).format(viewers);
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  const index = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    units.length - 1,
  );
  const value = (bytes / 1024 ** index)
    .toFixed(index === 0 ? 0 : 1)
    .replace(/\.0$/, '');
  return `${value} ${units[index]}`;
}

export function localDateTime(value?: string | null): string {
  if (!value) return 'an unknown time';
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return 'an unknown time';
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(timestamp);
}

export function youtubeUrl(videoId: string): string {
  return `https://www.youtube.com/watch?v=${videoId}`;
}

export function youtubeThumbnailUrl(value?: string | null): string | null {
  if (!value) return null;
  try {
    const url = new URL(value);
    const youtubeImageHost =
      url.hostname === 'img.youtube.com' || url.hostname.endsWith('.ytimg.com');
    if (!youtubeImageHost) return value;
    url.pathname = url.pathname.replace(
      /\/(?:default|hqdefault|sddefault)\.(jpg|webp)$/i,
      '/mqdefault.$1',
    );
    return url.href;
  } catch {
    return value;
  }
}

export function twitchUrl(login: string): string {
  return `https://www.twitch.tv/${login}`;
}

export function twitchChatUrl(login: string): string {
  return `https://www.twitch.tv/popout/${login}/chat?popout=`;
}

export function kickUrl(slug: string): string {
  return `https://kick.com/${slug}`;
}

export function isPlaybackLaunching(state: string): boolean {
  return ['launching', 'starting', 'connected'].includes(state);
}
