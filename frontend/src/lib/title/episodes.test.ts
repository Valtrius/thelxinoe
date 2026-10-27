import { describe, expect, it } from 'vitest';
import type { Episode, Season } from './catalog';
import type { TitleState } from './status';
import {
  episodeCode,
  phase,
  resumeText,
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
