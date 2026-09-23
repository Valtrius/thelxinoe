export type MediaKind = 'movie' | 'tv';
export type Media = {
  id: number;
  mediaType?: MediaKind | 'person';
  title?: string;
  name?: string;
  overview?: string;
  posterPath?: string | null;
  backdropPath?: string | null;
  releaseDate?: string;
  firstAirDate?: string;
  voteAverage?: number;
  runtime?: number;
  numberOfSeasons?: number;
  status?: string;
  tagline?: string;
  genres?: { id: number; name: string }[];
  productionCompanies?: { id: number; name: string }[];
  networks?: { id: number; name: string }[];
  credits?: {
    cast: {
      id: number;
      name: string;
      character?: string;
      profilePath?: string;
    }[];
  };
  relatedVideos?: { key: string; site: string; type: string; name: string }[];
  seasons?: {
    id: number;
    seasonNumber: number;
    name: string;
    episodeCount: number;
    airDate?: string;
    posterPath?: string;
  }[];
  mediaInfo?: {
    status: number;
    seasons?: { seasonNumber: number; status: number }[];
    requests?: MediaRequest[];
  };
};
export type MediaRequest = {
  id: number;
  status: number;
  createdAt: string;
  type: MediaKind;
  requestedBy?: { id: number; username?: string; displayName?: string };
  seasons?: { seasonNumber: number; status: number }[];
  media: { tmdbId: number; mediaType: MediaKind; status: number };
};
export type Results = {
  page: number;
  totalPages: number;
  totalResults: number;
  results: Media[];
};
export const title = (item: Media) => item.title || item.name || 'Untitled';
export const year = (item: Media) =>
  (item.releaseDate || item.firstAirDate || '').slice(0, 4);
export const kind = (item: Media): MediaKind =>
  item.mediaType === 'tv' || (!item.title && !!item.name) ? 'tv' : 'movie';
export function artwork(path?: string | null, size = 'w500') {
  return path && /^\/[a-zA-Z0-9_-]+\.(jpg|png|webp)$/.test(path)
    ? `https://image.tmdb.org/t/p/${size}${path}`
    : '';
}
export function availability(status?: number) {
  return (
    (
      {
        2: 'Pending approval',
        3: 'Requested',
        4: 'Partially available',
        5: 'Available',
        6: 'Unavailable',
      } as Record<number, string>
    )[status ?? 0] || ''
  );
}
export function seasonRequested(media: Media, number: number) {
  return (
    [5, 6].includes(media.mediaInfo?.status ?? 0) ||
    !!media.mediaInfo?.seasons?.some(
      (s) => s.seasonNumber === number && [2, 3, 4, 5].includes(s.status),
    ) ||
    !!media.mediaInfo?.requests?.some(
      (r) =>
        [1, 2, 4].includes(r.status) &&
        r.seasons?.some((s) => s.seasonNumber === number && s.status !== 3),
    )
  );
}
