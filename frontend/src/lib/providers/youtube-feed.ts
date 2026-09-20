import type { YoutubeGrouping, YoutubeVideo } from './types';

export interface YoutubeVideoGroup {
  key: string;
  label: string;
  videos: YoutubeVideo[];
}

const dayMs = 86_400_000;

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function startOfWeek(date: Date): Date {
  const day = startOfDay(date);
  day.setDate(day.getDate() - ((day.getDay() + 6) % 7));
  return day;
}

function formatDate(date: Date): string {
  return new Intl.DateTimeFormat(undefined, {
    weekday: 'long',
    day: 'numeric',
    month: 'short',
    year: 'numeric',
  }).format(date);
}

function formatRange(start: Date, end: Date): string {
  const sameMonth =
    start.getMonth() === end.getMonth() &&
    start.getFullYear() === end.getFullYear();
  const startLabel = new Intl.DateTimeFormat(undefined, {
    day: 'numeric',
    month: sameMonth ? undefined : 'short',
    year: start.getFullYear() === end.getFullYear() ? undefined : 'numeric',
  }).format(start);
  const endLabel = new Intl.DateTimeFormat(undefined, {
    day: 'numeric',
    month: 'short',
    year: 'numeric',
  }).format(end);
  return `${startLabel}–${endLabel}`;
}

function bucket(date: Date, grouping: YoutubeGrouping, today: Date): Date {
  if (grouping === 'day') return startOfDay(date);
  if (grouping === 'week') return startOfWeek(date);
  if (grouping === 'month')
    return new Date(date.getFullYear(), date.getMonth(), 1);
  if (grouping === 'smart') {
    const age = Math.floor(
      (startOfDay(today).getTime() - startOfDay(date).getTime()) / dayMs,
    );
    if (age <= 1) return startOfDay(date);
    if (age <= 56) return startOfWeek(date);
    return new Date(date.getFullYear(), date.getMonth(), 1);
  }
  return new Date(0);
}

function groupLabel(
  keyDate: Date,
  grouping: YoutubeGrouping,
  today: Date,
): string {
  const todayStart = startOfDay(today);
  if (grouping === 'day' || grouping === 'smart') {
    const age = Math.round((todayStart.getTime() - keyDate.getTime()) / dayMs);
    if (age === 0) return `Today · ${formatDate(keyDate)}`;
    if (age === 1) return `Yesterday · ${formatDate(keyDate)}`;
  }
  const age = Math.floor((todayStart.getTime() - keyDate.getTime()) / dayMs);
  if (grouping === 'week' || (grouping === 'smart' && age <= 56)) {
    const end = new Date(keyDate);
    end.setDate(end.getDate() + 6);
    const currentWeek = startOfWeek(todayStart).getTime();
    const prefix =
      keyDate.getTime() === currentWeek
        ? 'This week'
        : keyDate.getTime() === currentWeek - 7 * dayMs
          ? 'Last week'
          : 'Week';
    return `${prefix} · ${formatRange(keyDate, end)}`;
  }
  if (grouping === 'month' || grouping === 'smart') {
    return new Intl.DateTimeFormat(undefined, {
      month: 'long',
      year: 'numeric',
    }).format(keyDate);
  }
  return formatDate(keyDate);
}

export function groupYoutubeVideos(
  videos: YoutubeVideo[],
  grouping: YoutubeGrouping,
  now = new Date(),
): YoutubeVideoGroup[] {
  const live = videos.filter((video) => video.broadcastState === 'live');
  const recorded = videos.filter((video) => video.broadcastState !== 'live');
  const liveGroup: YoutubeVideoGroup[] = live.length
    ? [{ key: 'live', label: 'Live now', videos: live }]
    : [];
  if (grouping === 'none') {
    return liveGroup.length
      ? [...liveGroup, { key: 'all', label: '', videos: recorded }].filter(
          (group) => group.videos.length > 0,
        )
      : [{ key: 'all', label: '', videos }];
  }
  const groups = new Map<string, YoutubeVideoGroup>();
  for (const video of recorded) {
    const keyDate = bucket(new Date(video.publishedAt), grouping, now);
    const key = keyDate.toISOString();
    const current = groups.get(key);
    if (current) current.videos.push(video);
    else
      groups.set(key, {
        key,
        label: groupLabel(keyDate, grouping, now),
        videos: [video],
      });
  }
  return [...liveGroup, ...groups.values()];
}
