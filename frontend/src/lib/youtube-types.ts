export type Video = {
  id: string;
  title: string;
  channel: string;
  published_at: number;
  duration: number | null;
  broadcast: string;
  available: boolean;
  is_short: boolean | null;
  pending: boolean;
  watchlist: boolean;
  pinned: boolean;
  watched: boolean;
  position: number;
  artwork_url?: string;
  download?: string;
};
export function duration(seconds: number | null) {
  if (seconds === null) return '';
  const h = Math.floor(seconds / 3600),
    m = Math.floor(seconds / 60) % 60,
    s = Math.floor(seconds % 60);
  return h
    ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
    : `${m}:${String(s).padStart(2, '0')}`;
}
