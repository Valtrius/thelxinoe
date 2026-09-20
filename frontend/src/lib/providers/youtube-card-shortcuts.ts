import type { YoutubeCardShortcut } from './types';

export const MAX_YOUTUBE_CARD_SHORTCUTS = 3;

export const youtubeCardShortcutOptions = [
  { id: 'play_from_beginning', label: 'Play from beginning' },
  { id: 'download', label: 'Download' },
  { id: 'watch_toggle', label: 'Mark watched or unwatched' },
  { id: 'copy_url', label: 'Copy YouTube URL' },
  { id: 'open_browser', label: 'Open in browser' },
  { id: 'pin_download', label: 'Pin or unpin download' },
  { id: 'delete_download', label: 'Delete download' },
  { id: 'add_watch_later', label: 'Add or remove from Watch Later' },
] as const satisfies ReadonlyArray<{
  id: YoutubeCardShortcut;
  label: string;
}>;

export function toggleYoutubeCardShortcut(
  shortcuts: YoutubeCardShortcut[],
  shortcut: YoutubeCardShortcut,
): YoutubeCardShortcut[] {
  if (shortcuts.includes(shortcut)) {
    return shortcuts.filter((current) => current !== shortcut);
  }
  if (shortcuts.length >= MAX_YOUTUBE_CARD_SHORTCUTS) return shortcuts;
  return [...shortcuts, shortcut];
}
