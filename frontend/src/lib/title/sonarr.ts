import { api } from '../api';

// Administrator actions on a series' Sonarr episodes. The series is the library id or
// tmdb:/tvdb: reference its seasons load from; episodes are Sonarr ids.
const path = (series: string) => `/admin/series/${encodeURIComponent(series)}`;

export type SearchTarget =
  | { episodes: number[]; monitor?: boolean }
  | { season: number; missing?: boolean };
export const searchEpisodes = (series: string, target: SearchTarget) =>
  api<{ episodes: number | null }>(`${path(series)}/search`, 'POST', target);

export const monitorEpisodes = (
  series: string,
  episodes: number[],
  monitored: boolean,
) => api(`${path(series)}/monitor`, 'PUT', { episodes, monitored });

export type ManagerEpisode = { monitored: boolean; last_search: number | null };
export const loadManagerEpisode = (series: string, episode: number) =>
  api<ManagerEpisode>(`${path(series)}/episodes/${episode}`);

export type Release = {
  guid: string;
  indexer_id: number;
  indexer: string | null;
  title: string;
  size: number | null;
  protocol: string | null;
  quality: string | null;
  score: number | null;
  seeders: number | null;
  leechers: number | null;
  published: number | null;
  full_season: boolean;
  approved: boolean;
  rejections: string[];
};
export type ReleaseTarget = { episode: number } | { season: number };
export const loadReleases = (series: string, target: ReleaseTarget) =>
  api<{ items: Release[] }>(
    `${path(series)}/releases?${'episode' in target ? `episode=${target.episode}` : `season=${target.season}`}`,
  ).then((result) => result.items);
export const grabRelease = (series: string, release: Release) =>
  api(`${path(series)}/releases`, 'POST', {
    guid: release.guid,
    indexer_id: release.indexer_id,
  });
