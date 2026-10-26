import { formatBytes } from './providers/utils';
import { day } from './title/status';

export type LibrarySort =
  | 'title'
  | 'year'
  | 'added'
  | 'rating'
  | 'size'
  | 'runtime'
  | 'released'
  | 'episodes'
  | 'last_aired'
  | 'next_airing';
export type SortOrder = 'asc' | 'desc';
export type WatchState = 'unwatched' | 'in_progress' | 'watched';
export type Availability =
  'downloaded' | 'downloading' | 'upcoming' | 'not_downloaded';
export type SeriesState = 'any' | 'continuing' | 'ended';

/** How a user sorts and filters their movies or shows. */
export type LibraryView = {
  sort: LibrarySort;
  order: SortOrder;
  watch: WatchState[];
  availability: Availability[];
  series: SeriesState;
  genre: string;
};

/** What each filter would keep, across what the search and collection match. */
export type LibraryCounts = {
  watch: Record<WatchState, number>;
  availability: Record<Availability, number>;
  series?: Record<Exclude<SeriesState, 'any'>, number>;
};

/** A card's values for the sorts it shows, as the list's server sends them. */
export type SortValues = Partial<
  Record<Exclude<LibrarySort, 'title' | 'year'>, number>
>;

const movieSorts: [LibrarySort, string][] = [
  ['title', 'Title'],
  ['year', 'Year'],
  ['released', 'Release date'],
  ['added', 'Date added'],
  ['rating', 'Rating'],
  ['runtime', 'Runtime'],
  ['size', 'Size on disk'],
];
const showSorts: [LibrarySort, string][] = [
  ['title', 'Title'],
  ['year', 'Year'],
  ['last_aired', 'Last aired'],
  ['next_airing', 'Next airing'],
  ['added', 'Date added'],
  ['rating', 'Rating'],
  ['episodes', 'Episodes'],
  ['size', 'Size on disk'],
];
export const sortsFor = (shows: boolean) => (shows ? showSorts : movieSorts);
export const watchStates: [WatchState, string][] = [
  ['unwatched', 'Unwatched'],
  ['in_progress', 'In progress'],
  ['watched', 'Watched'],
];
export const availabilities: [Availability, string][] = [
  ['downloaded', 'Downloaded'],
  ['downloading', 'Downloading'],
  ['upcoming', 'Upcoming'],
  ['not_downloaded', 'Not downloaded'],
];
export const seriesStates: [SeriesState, string][] = [
  ['any', 'Any'],
  ['continuing', 'Continuing'],
  ['ended', 'Ended'],
];

export const libraryViewKey = (domain: string) =>
  `library-${domain.toLowerCase()}-view`;

export function defaultLibraryView(): LibraryView {
  return {
    sort: 'title',
    order: 'asc',
    watch: watchStates.map(([state]) => state),
    availability: availabilities.map(([state]) => state),
    series: 'any',
    genre: '',
  };
}

function choice<T extends string>(
  value: unknown,
  allowed: [T, string][],
  fallback: T,
): T {
  return allowed.some(([option]) => option === value) ? (value as T) : fallback;
}
function choices<T extends string>(
  value: unknown,
  allowed: [T, string][],
  fallback: T[],
): T[] {
  if (!Array.isArray(value)) return fallback;
  const chosen = allowed
    .map(([option]) => option)
    .filter((option) => value.includes(option));
  return chosen.length ? chosen : fallback;
}

export function parseLibraryView(
  serialized: string | null | undefined,
  shows: boolean,
): LibraryView {
  const fallback = defaultLibraryView();
  if (!serialized) return fallback;
  try {
    const saved = JSON.parse(serialized) as Record<string, unknown>;
    return {
      sort: choice(saved.sort, sortsFor(shows), fallback.sort),
      order: saved.order === 'desc' ? 'desc' : 'asc',
      watch: choices(saved.watch, watchStates, fallback.watch),
      availability: choices(
        saved.availability,
        availabilities,
        fallback.availability,
      ),
      series: choice(saved.series, seriesStates, fallback.series),
      genre:
        typeof saved.genre === 'string'
          ? saved.genre.slice(0, 100)
          : fallback.genre,
    };
  } catch {
    return fallback;
  }
}

/** The view without its filters, sorted as before. */
export const unfiltered = (view: LibraryView): LibraryView => ({
  ...defaultLibraryView(),
  sort: view.sort,
  order: view.order,
});

/**
 * Picking an option while all are chosen keeps only it; later picks add or remove
 * one, and removing the last returns to all of them.
 */
export function choose<T extends string>(
  chosen: T[],
  options: [T, string][],
  option: T,
): T[] {
  const all = options.map(([value]) => value);
  if (chosen.length === all.length) return [option];
  const next = chosen.includes(option)
    ? chosen.filter((value) => value !== option)
    : [...chosen, option];
  return next.length ? all.filter((value) => next.includes(value)) : all;
}

/** Whether the view hides anything; sorting hides nothing. */
export function filtering(view: LibraryView, shows: boolean) {
  return (
    view.watch.length < watchStates.length ||
    view.availability.length < availabilities.length ||
    (shows && view.series !== 'any') ||
    view.genre !== ''
  );
}

/** The catalog query parameters of a view, leaving out what the server defaults to. */
export function viewQuery(view: LibraryView, shows: boolean) {
  const query = new URLSearchParams();
  if (view.sort !== 'title' || view.order !== 'asc') {
    query.set('sort', view.sort);
    query.set('order', view.order);
  }
  if (view.watch.length < watchStates.length)
    query.set('watch', view.watch.join(','));
  if (view.availability.length < availabilities.length)
    query.set('availability', view.availability.join(','));
  if (shows && view.series !== 'any') query.set('series', view.series);
  if (view.genre) query.set('genre', view.genre);
  return query.toString();
}

/** What a card shows of the value its list is sorted by; every card shows its year. */
export function sortDetail(
  sort: LibrarySort,
  values: SortValues,
  timezone: string,
  now = Date.now(),
): string | undefined {
  if (sort === 'title' || sort === 'year') return undefined;
  const value = values[sort];
  if (value == null) return undefined;
  const date = () => day(value, timezone, now);
  switch (sort) {
    case 'added':
      return `Added ${date()}`;
    case 'rating':
      return `Rated ${value.toFixed(1)}`;
    case 'size':
      return formatBytes(value);
    case 'runtime':
      return `${value} min`;
    case 'released':
      return `${value * 1000 > now ? 'Releases' : 'Released'} ${date()}`;
    case 'episodes':
      return `${value} ${value === 1 ? 'episode' : 'episodes'}`;
    case 'last_aired':
      return `Aired ${date()}`;
    case 'next_airing':
      return `Airs ${date()}`;
  }
}
