import { describe, expect, it } from 'vitest';
import { hasStatus, isTitleRef, statusChip, statusText } from './status';

const now = Date.UTC(2026, 9, 17, 12);
const at = (iso: string) => Date.parse(iso) / 1000;

describe('title status', () => {
  it('describes downloads, imports and remaining time', () => {
    expect(
      statusText(
        {
          state: 'downloading',
          progress: 0.426,
          eta: now / 1000 + 12 * 60,
        },
        'UTC',
        now,
      ),
    ).toEqual({
      label: 'Downloading 42%',
      detail: '12 min left',
      tone: 'accent',
    });
    expect(
      statusText({ state: 'downloading', progress: 1 }, 'UTC', now).label,
    ).toBe('Importing');
    expect(
      statusText(
        { state: 'downloading', progress: 0.5, downloading: 3 },
        'UTC',
        now,
      ).label,
    ).toBe('Downloading 3 episodes · 50%');
  });

  it('dates upcoming releases and next episodes', () => {
    expect(
      statusText(
        { state: 'upcoming', release: at('2026-11-14T00:00:00Z') },
        'UTC',
        now,
      ).label,
    ).toMatch(/14/);
    expect(statusText({ state: 'upcoming' }, 'UTC', now).label).toBe(
      'Coming soon',
    );
    const partial = statusText(
      {
        state: 'partial',
        episodes: { available: 8, total: 10 },
        release: at('2026-10-24T20:00:00Z'),
      },
      'UTC',
      now,
    );
    expect(partial.label).toBe('8 of 10 episodes');
    expect(partial.detail).toMatch(/^Next episode /);
  });

  it('keeps admin-only states distinct from the user wording', () => {
    expect(statusText({ state: 'missing' }, 'UTC', now).label).toBe('Missing');
    expect(statusText({ state: 'unmonitored' }, 'UTC', now).label).toBe(
      'Not monitored',
    );
    expect(statusText({ state: 'unavailable' }, 'UTC', now).label).toBe(
      'Not available',
    );
  });

  it('counts episodes on series posters and shows nothing for idle titles', () => {
    expect(statusChip({ state: 'available' }, 'UTC', now)).toBeNull();
    expect(statusChip(null, 'UTC', now)).toBeNull();
    expect(
      statusChip(
        { state: 'available', episodes: { available: 11, total: 11 } },
        'UTC',
        now,
      ),
    ).toEqual({ label: '11/11', tone: 'success' });
    expect(
      statusChip({ state: 'idle', inactive: true }, 'UTC', now),
    ).toBeNull();
    expect(hasStatus({ state: 'idle' })).toBe(false);
    expect(hasStatus({ state: 'unmonitored' })).toBe(true);
    expect(
      statusChip(
        { state: 'partial', episodes: { available: 3, total: 9 } },
        'UTC',
        now,
      )?.label,
    ).toBe('3/9');
    expect(
      statusChip({ state: 'downloading', progress: 0.999 }, 'UTC', now)?.label,
    ).toBe('99%');
  });

  it('recognises Radarr and Sonarr title references', () => {
    expect(isTitleRef('tmdb:603')).toBe(true);
    expect(isTitleRef('tvdb:81189')).toBe(true);
    expect(isTitleRef('movie-0')).toBe(false);
    expect(isTitleRef('imdb:tt0133093')).toBe(false);
  });
});
