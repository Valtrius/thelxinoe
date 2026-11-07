import { describe, expect, it } from 'vitest';
import type { Episode, Season } from './catalog';
import type { TitleState } from './status';
import {
  episodeCode,
  episodeToPlay,
  phase,
  resumeText,
  seasonFiles,
  seasonToOpen,
  shown,
  summarize,
  wanted,
} from './episodes';

const NOW = 1_800_000_000;
const episode = (
  number: number,
  state: TitleState | null,
  extra: Partial<Episode> = {},
): Episode => ({
  id: null,
  season: 2,
  number,
  title: `Episode ${number}`,
  overview: null,
  air_date: NOW - 86400,
  runtime: 45,
  available: false,
  watched: false,
  position: null,
  duration: null,
  status: state ? { state } : null,
  stills: [],
  ...extra,
});
const season = (episodes: Episode[]): Season => ({
  number: 2,
  id: 'season',
  title: 'Season 2',
  status: null,
  watched: false,
  episodes,
});

describe('episode phases', () => {
  it('put files first, then downloads, air dates and Sonarr states', () => {
    expect(phase(episode(1, 'missing', { available: true }), NOW)).toBe(
      'available',
    );
    expect(phase(episode(1, 'available'), NOW)).toBe('available');
    expect(phase(episode(1, 'queued'), NOW)).toBe('downloading');
    expect(phase(episode(1, 'downloading'), NOW)).toBe('downloading');
    expect(phase(episode(1, 'missing'), NOW)).toBe('missing');
    // An unmonitored episode that has not aired yet still comes on air.
    expect(phase(episode(1, 'unmonitored', { air_date: NOW + 60 }), NOW)).toBe(
      'upcoming',
    );
    expect(phase(episode(1, 'unmonitored'), NOW)).toBe('unmonitored');
    expect(phase(episode(1, 'unavailable'), NOW)).toBe('unavailable');
    // A library episode without Sonarr or a file is simply not available.
    expect(phase(episode(1, null), NOW)).toBe('unavailable');
    expect(phase(episode(1, null, { air_date: null }), NOW)).toBe(
      'unavailable',
    );
  });

  it('count what a season holds and when an unaired one premieres', () => {
    const summary = summarize(
      season([
        episode(1, 'available', { watched: true, available: true }),
        episode(2, null, { watched: true, available: true }),
        episode(3, 'downloading'),
        episode(4, 'missing'),
        episode(5, 'unmonitored'),
        episode(6, 'upcoming', { air_date: NOW + 86400 }),
      ]),
      NOW,
    );
    expect(summary).toEqual({
      total: 6,
      watched: 2,
      available: 2,
      downloading: 1,
      missing: 2,
      unavailable: 0,
      upcoming: 1,
      premiere: null,
    });
    expect(
      summarize(
        season([
          episode(2, 'upcoming', { air_date: NOW + 200 }),
          episode(1, 'upcoming', { air_date: NOW + 100 }),
          episode(3, 'upcoming', { air_date: null }),
        ]),
        NOW,
      ).premiere,
    ).toBe(NOW + 100);
  });

  it('tell how much of a season is on disk, counting what has not aired', () => {
    const here = (number: number) =>
      episode(number, 'available', { available: true });
    const coming = (number: number) =>
      episode(number, 'upcoming', { air_date: NOW + 86400 });
    const files = (episodes: Episode[]) =>
      seasonFiles(summarize(season(episodes), NOW));

    expect(files([here(1), here(2)])).toBe('complete');
    expect(files([here(1), coming(2)])).toBe('ongoing');
    expect(files([coming(1), coming(2)])).toBe('upcoming');
    expect(files([here(1), episode(2, 'missing')])).toBe('partial');
    expect(files([here(1), episode(2, 'missing'), coming(3)])).toBe('partial');
    expect(files([here(1), episode(2, 'downloading'), coming(3)])).toBe(
      'downloading',
    );
    expect(files([episode(1, 'missing'), coming(2)])).toBe('empty');
    expect(files([episode(1, 'unavailable')])).toBe('empty');
    expect(files([])).toBe('empty');
  });

  it('filter unwatched episodes and the aired ones administrators can fetch', () => {
    const watched = episode(1, 'available', { watched: true });
    const missing = episode(2, 'unmonitored');
    const coming = episode(3, 'upcoming', { air_date: NOW + 10 });
    expect(
      [watched, missing, coming].filter((e) => shown(e, 'unwatched', NOW)),
    ).toEqual([missing, coming]);
    expect(
      [watched, missing, coming].filter((e) => shown(e, 'missing', NOW)),
    ).toEqual([missing]);
    expect(wanted(episode(4, 'missing'), NOW)).toBe(true);
    expect(wanted(episode(4, 'unavailable'), NOW)).toBe(false);
  });

  it('describe where an episode resumes and name it by its numbers', () => {
    expect(
      resumeText(episode(1, null, { position: 2050, duration: 3120 })),
    ).toBe('Resume at 34:10 · 18 min left');
    expect(resumeText(episode(1, null, { position: 30, duration: 60 }))).toBe(
      'Resume at 0:30 · 1 min left',
    );
    expect(
      resumeText(
        episode(1, null, { position: 2050, duration: 3120, watched: true }),
      ),
    ).toBe('');
    expect(resumeText(episode(1, null))).toBe('');
    expect(episodeCode(episode(6, null))).toBe('S02E06');
  });
});

describe('going on with a series', () => {
  const here = (id: string, extra: Partial<Episode> = {}) =>
    episode(Number(id.slice(-1)), null, {
      id,
      season: Number(id.slice(1, 2)),
      available: true,
      ...extra,
    });
  const absent = (id: string, extra: Partial<Episode> = {}) =>
    here(id, { id: null, available: false, ...extra });
  const seasons = (...lists: Episode[][]): Season[] =>
    lists.map((episodes, index) => ({
      ...season(episodes),
      number: index + 1,
      title: `Season ${index + 1}`,
    }));
  const played = { watched: true };
  const specials: Season = {
    ...season([here('s01', { season: 0 })]),
    number: 0,
    title: 'Specials',
  };

  it('starts from the first available episode when nothing was watched', () => {
    const list = seasons(
      [absent('s11'), absent('s12')],
      [here('s21'), here('s22')],
    );
    expect(episodeToPlay(list)?.id).toBe('s21');
    expect(seasonToOpen(list)).toBe(2);
  });

  it('follows the last watched episode, past the ones it skipped', () => {
    const list = seasons(
      [here('s11'), here('s12', { position: 300 }), here('s13', played)],
      [here('s21'), here('s22')],
    );
    expect(episodeToPlay(list)?.id).toBe('s21');
    expect(seasonToOpen(list)).toBe(2);
  });

  it('resumes the episode in progress after the last watched', () => {
    const list = seasons(
      [here('s11', played), here('s12'), here('s13', { position: 300 })],
      [here('s21')],
    );
    expect(episodeToPlay(list)?.id).toBe('s13');
    expect(seasonToOpen(list)).toBe(1);
  });

  it('reaches over an absent episode to the next available one', () => {
    const list = seasons([here('s11', played), absent('s12')], [here('s21')]);
    expect(episodeToPlay(list)?.id).toBe('s21');
    expect(seasonToOpen(list)).toBe(2);
  });

  it('opens the next season before its files are here, while Play looks back', () => {
    const list = seasons(
      [here('s11', { position: 300 }), here('s12', played)],
      [absent('s21'), absent('s22')],
    );
    expect(episodeToPlay(list)?.id).toBe('s11');
    expect(seasonToOpen(list)).toBe(2);

    const caught = seasons([here('s11', played)], [absent('s21')]);
    expect(episodeToPlay(caught)?.id).toBe('s11');
    expect(seasonToOpen(caught)).toBe(2);
  });

  it('opens the last season once everything was watched, and Play restarts', () => {
    const list = seasons(
      [here('s11', played)],
      [here('s21', played), here('s22', played)],
    );
    expect(episodeToPlay(list)?.id).toBe('s11');
    expect(seasonToOpen(list)).toBe(2);
  });

  it('keeps specials out of the order and opens a series without files at its start', () => {
    const list = [
      ...seasons([absent('s11')], [absent('s21')]),
      { ...specials, episodes: [here('s01', { season: 0, ...played })] },
    ];
    expect(episodeToPlay(list)).toBeUndefined();
    expect(seasonToOpen(list)).toBe(1);
    expect(episodeToPlay([specials])).toBeUndefined();
    expect(seasonToOpen([specials])).toBe(0);
    expect(seasonToOpen([])).toBeNull();
  });
});
