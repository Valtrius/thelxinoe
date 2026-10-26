export type LibrarySort = 'title' | 'year' | 'added' | 'rating';
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

export const sorts: [LibrarySort, string][] = [
  ['title', 'Title'],
  ['year', 'Year'],
  ['added', 'Date added'],
  ['rating', 'Rating'],
];
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
): LibraryView {
  const fallback = defaultLibraryView();
  if (!serialized) return fallback;
  try {
    const saved = JSON.parse(serialized) as Record<string, unknown>;
    return {
      sort: choice(saved.sort, sorts, fallback.sort),
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
