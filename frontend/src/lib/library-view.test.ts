import { expect, it } from 'vitest';
import {
  availabilities,
  choose,
  defaultLibraryView,
  filtering,
  parseLibraryView,
  sortDetail,
  viewQuery,
} from './library-view';

it('asks the server only for what differs from its defaults', () => {
  const view = defaultLibraryView();
  expect(viewQuery(view, true)).toBe('');
  expect(filtering(view, true)).toBe(false);
  const changed = {
    ...view,
    sort: 'added' as const,
    order: 'desc' as const,
    watch: ['unwatched' as const, 'in_progress' as const],
    availability: ['not_downloaded' as const],
    series: 'ended' as const,
    genre: 'Science Fiction',
  };
  expect(viewQuery(changed, true)).toBe(
    'sort=added&order=desc&watch=unwatched%2Cin_progress&availability=not_downloaded&series=ended&genre=Science+Fiction',
  );
  // Movies have no series state.
  expect(viewQuery({ ...view, series: 'ended' }, false)).toBe('');
  expect(filtering({ ...view, series: 'ended' }, false)).toBe(false);
  expect(filtering({ ...view, sort: 'year' }, false)).toBe(false);
  expect(filtering({ ...view, genre: 'Drama' }, false)).toBe(true);
});

it('keeps what it recognizes of a saved view', () => {
  expect(parseLibraryView(null, false)).toEqual(defaultLibraryView());
  expect(parseLibraryView('not json', false)).toEqual(defaultLibraryView());
  expect(
    parseLibraryView(
      JSON.stringify({
        sort: 'length',
        order: 'desc',
        watch: ['watched', 'seen'],
        availability: [],
        series: 'ended',
        genre: 7,
      }),
      true,
    ),
  ).toEqual({
    ...defaultLibraryView(),
    order: 'desc',
    watch: ['watched'],
    series: 'ended',
  });
  // Each section keeps only its own sorts.
  const airing = JSON.stringify({ sort: 'next_airing' });
  expect(parseLibraryView(airing, true).sort).toBe('next_airing');
  expect(parseLibraryView(airing, false).sort).toBe('title');
});

it('shows the value a card is sorted by, unless every card already does', () => {
  const now = Date.UTC(2026, 9, 26, 12);
  const card = {
    added: Date.UTC(2026, 9, 12, 12) / 1000,
    rating: 7.84,
    size: 45.3 * 1024 ** 3,
    runtime: 134,
    released: Date.UTC(2023, 4, 12, 12) / 1000,
    episodes: 1,
    next_airing: Date.UTC(2026, 10, 3, 2) / 1000,
  };
  const detail = (sort: Parameters<typeof sortDetail>[0]) =>
    sortDetail(sort, card, 'UTC', now);
  expect(detail('title')).toBeUndefined();
  expect(detail('year')).toBeUndefined();
  expect(detail('rating')).toBe('Rated 7.8');
  expect(detail('size')).toBe('45.3 GB');
  expect(detail('runtime')).toBe('134 min');
  expect(detail('episodes')).toBe('1 episode');
  expect(detail('last_aired')).toBeUndefined();
  // Dates leave out the current year.
  expect(detail('added')).toMatch(/^Added (Oct 12|12 Oct)$/);
  expect(detail('released')).toMatch(/^Released .*2023$/);
  expect(detail('next_airing')).toMatch(/^Airs (Nov 3|3 Nov)$/);
  // The airing time falls on the day before in New York.
  expect(sortDetail('next_airing', card, 'America/New_York', now)).toMatch(
    /^Airs (Nov 2|2 Nov)$/,
  );
  expect(
    sortDetail('released', { released: now / 1000 + 86400 }, 'UTC', now),
  ).toMatch(/^Releases /);
});

it('narrows from all to one choice and widens back once nothing is left', () => {
  const all = defaultLibraryView().availability;
  const only = choose(all, availabilities, 'not_downloaded');
  expect(only).toEqual(['not_downloaded']);
  // Choices keep the server's order however they were picked.
  expect(choose(only, availabilities, 'downloaded')).toEqual([
    'downloaded',
    'not_downloaded',
  ]);
  expect(choose(only, availabilities, 'not_downloaded')).toEqual(all);
});
