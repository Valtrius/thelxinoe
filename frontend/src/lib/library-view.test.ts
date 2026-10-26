import { expect, it } from 'vitest';
import {
  availabilities,
  choose,
  defaultLibraryView,
  filtering,
  parseLibraryView,
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
  expect(parseLibraryView(null)).toEqual(defaultLibraryView());
  expect(parseLibraryView('not json')).toEqual(defaultLibraryView());
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
    ),
  ).toEqual({
    ...defaultLibraryView(),
    order: 'desc',
    watch: ['watched'],
    series: 'ended',
  });
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
