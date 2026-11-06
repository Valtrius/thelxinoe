export type View = 'artists' | 'albums' | 'tracks';
export type Sort = 'title' | 'year' | 'added';
export type Order = 'asc' | 'desc';
export type ListSort = { sort: Sort; order: Order };
type Preferences = Record<string, string>;

export const views: [View, string, string][] = [
  ['artists', 'Artists', 'artist'],
  ['albums', 'Albums', 'album'],
  ['tracks', 'Tracks', 'track'],
];
export const sorts: Record<View, [Sort, string][]> = {
  artists: [
    ['title', 'Name'],
    ['added', 'Date added'],
  ],
  albums: [
    ['title', 'Title'],
    ['year', 'Year'],
    ['added', 'Date added'],
  ],
  tracks: [
    ['title', 'Title'],
    ['added', 'Date added'],
  ],
};

export const viewKey = 'music-view';
const sortKey = 'music-sort';

const isView = (value?: string): value is View =>
  views.some(([known]) => known === value);

// An address without a list opens the one the user last chose.
export function listView(route: string | undefined, saved: Preferences): View {
  const last = saved[viewKey];
  return isView(route) ? route : isView(last) ? last : 'artists';
}

export const kindOf = (view: View) =>
  views.find(([value]) => value === view)![2];

function savedSorts(saved: Preferences) {
  try {
    return JSON.parse(saved[sortKey] ?? '{}') as Partial<
      Record<View, ListSort>
    >;
  } catch {
    return {};
  }
}

// Each list keeps the user's sort.
export function listSort(view: View, saved: Preferences): ListSort {
  const sort = savedSorts(saved)[view];
  return sort && sorts[view].some(([value]) => value === sort.sort)
    ? sort
    : { sort: 'title', order: 'asc' };
}

export const savedSort = (view: View, next: ListSort, saved: Preferences) => ({
  [sortKey]: JSON.stringify({ ...savedSorts(saved), [view]: next }),
});
