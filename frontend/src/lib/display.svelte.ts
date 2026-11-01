import { dateFormat } from './intl';

export type TimeFormat = '12h' | '24h';

// The signed-in user's effective display preferences.
export const display = $state({
  timezone: 'UTC',
  timeFormat: '24h' as TimeFormat,
});

export function resetDisplay() {
  display.timezone = 'UTC';
  display.timeFormat = '24h';
}

/** Server times in seconds. Pass the server's zone for its own schedules. */
export function formatTimestamp(
  seconds: number,
  timeZone: string = display.timezone,
) {
  return dateFormat(undefined, {
    year: 'numeric',
    month: 'numeric',
    day: 'numeric',
    hour: 'numeric',
    minute: 'numeric',
    second: 'numeric',
    timeZone,
    hour12: display.timeFormat === '12h',
  }).format(seconds * 1000);
}

export function formatTime(
  seconds: number,
  timeZone: string = display.timezone,
) {
  return dateFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    timeZone,
    hour12: display.timeFormat === '12h',
  }).format(seconds * 1000);
}
