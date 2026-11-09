import { describe, expect, it } from 'vitest';
import type { Repeat } from '../gapless';
import type { MediaChoice } from '../playback';
import type { MusicItem } from './library';
import { restOf } from './library';
import {
  appendedQueue,
  extendedQueue,
  followedQueue,
  insertedQueue,
  playsAfter,
  removedQueue,
} from './queue-follow';

const list = (ids: string) =>
  [...ids].map((id): MediaChoice => ({ id, title: id }));
const ids = (choices: MediaChoice[]) => choices.map((c) => c.id).join('');
const backwards = <T>(items: T[]) => [...items].reverse();

function follow(
  from: string,
  index: number,
  to: string,
  shuffled = false,
  choices = list(from),
) {
  return followedQueue({ choices, index, shuffled }, list(to), backwards);
}

describe('extendedQueue', () => {
  const extend = (
    from: string,
    index: number,
    more: string,
    shuffled = false,
  ) =>
    extendedQueue(
      {
        choices: list(from),
        order: shuffled ? list([...from].sort().join('')) : list(from),
        index,
        shuffled,
      },
      list(more),
      backwards,
    );

  it('adds the rest of the list in order', () => {
    const next = extend('abc', 1, 'de');
    expect(ids(next.choices)).toBe('abcde');
    expect(ids(next.order)).toBe('abcde');
  });

  it('keeps a track added by hand ahead of the rest of the list', () => {
    expect(ids(extend('abxc', 0, 'de').choices)).toBe('abxcde');
  });

  it('mixes the rest into a shuffled queue after the next track', () => {
    const next = extend('cabd', 0, 'ef', true);
    expect(ids(next.choices)).toBe('cafedb');
    expect(ids(next.order)).toBe('abcdef');
  });

  it('continues a shuffled queue that ran out', () => {
    expect(ids(extend('ba', 2, 'cd', true).choices)).toBe('badc');
  });
});

describe('followedQueue', () => {
  it('moves the playing track to its new place', () => {
    const next = follow('abcd', 1, 'cbad');
    expect(ids(next.choices)).toBe('cbad');
    expect(next.index).toBe(1);
    expect(next.order).toBe(next.choices);
  });

  it('drops hand-added tracks and restores hand-removed ones', () => {
    const next = follow('axc', 0, 'abc');
    expect(ids(next.choices)).toBe('abc');
    expect(next.index).toBe(0);
  });

  it('keeps the choice of the playing track', () => {
    const choices = list('abc');
    const next = follow('abc', 1, 'bca', false, choices);
    expect(next.choices[0]).toBe(choices[1]);
  });

  it('lets a removed playing track finish, then what followed it', () => {
    const next = follow('abcd', 1, 'acd');
    expect(ids(next.choices)).toBe('abcd');
    expect(next.index).toBe(1);
  });

  it('skips followers that left with the playing track', () => {
    const next = follow('abcd', 1, 'ad');
    expect(ids(next.choices)).toBe('abd');
    expect(next.index).toBe(1);
  });

  it('plays a hand-removed track that comes back after a removed playing one', () => {
    const next = follow('abd', 1, 'acd');
    expect(ids(next.choices)).toBe('abcd');
    expect(next.index).toBe(1);
  });

  it('plays what was added after a removed last track', () => {
    const next = follow('abc', 2, 'abd');
    expect(ids(next.choices)).toBe('abcd');
    expect(next.index).toBe(2);
  });

  it('keeps a removed last track at the end', () => {
    const next = follow('abc', 2, 'ab');
    expect(ids(next.choices)).toBe('abc');
    expect(next.index).toBe(2);
  });

  it('does not reshuffle when a shuffled queue is only reordered', () => {
    const choices = list('cabd');
    const next = followedQueue(
      { choices, index: 1, shuffled: true },
      list('dcba'),
      backwards,
    );
    expect(next.choices).toBe(choices);
    expect(next.index).toBe(1);
    expect(ids(next.order)).toBe('dcba');
  });

  it('reshuffles only what is upcoming when tracks change', () => {
    const next = followedQueue(
      { choices: list('cabd'), index: 1, shuffled: true },
      list('abcde'),
      backwards,
    );
    expect(ids(next.choices)).toBe('caedb');
    expect(next.index).toBe(1);
    expect(ids(next.order)).toBe('abcde');
  });

  it('forgets played tracks that left the playlist', () => {
    const next = followedQueue(
      { choices: list('cabd'), index: 2, shuffled: true },
      list('abd'),
      backwards,
    );
    expect(ids(next.choices)).toBe('abd');
    expect(next.index).toBe(1);
  });

  it('keeps a removed playing track in a shuffled queue', () => {
    const next = followedQueue(
      { choices: list('cabd'), index: 1, shuffled: true },
      list('bcd'),
      backwards,
    );
    expect(ids(next.choices)).toBe('cabd');
    expect(next.index).toBe(1);
  });
});

describe('playsAfter', () => {
  it('repeats one track unless its playlist lost it', () => {
    const choices = list('abc');
    expect(playsAfter({ choices, repeat: 'one' }, 1)).toBe(1);
    expect(playsAfter({ choices, repeat: 'one', leaving: choices[1] }, 1)).toBe(
      2,
    );
  });

  it('does not start over with only a track that is leaving', () => {
    const choices = list('b');
    expect(playsAfter({ choices, repeat: 'all' }, 0)).toBe(0);
    expect(playsAfter({ choices, repeat: 'all', leaving: choices[0] }, 0)).toBe(
      1,
    );
  });
});

describe('insertedQueue', () => {
  const insert = (index: number, next: boolean, finished = false) =>
    insertedQueue(
      {
        choices: list('abc'),
        order: list('abc'),
        index,
        shuffled: false,
        finished,
      },
      list('x'),
      next,
    );

  it('adds after the playing track or at the end', () => {
    expect(ids(insert(0, true).choices)).toBe('axbc');
    expect(ids(insert(0, false).choices)).toBe('abcx');
  });

  it('adds to a finished queue where it ended', () => {
    const next = insert(3, true, true);
    expect(ids(next.choices)).toBe('abcx');
    expect(next.at).toBe(3);
  });
});

// The queue a list starts with is its first page; the rest of the list is read
// while the chosen track plays.
describe('the rest of a list', () => {
  const all = [...'abcdefg'].map((id): MusicItem => ({
    id,
    kind: 'track',
    title: id,
    year: null,
    available: true,
    metadata: {},
  }));
  const shown = list('abc');
  const rest = restOf(all, shown);
  const playing = (
    index: number,
    next: number,
    repeat: Repeat = 'off',
    choices = list('abc'),
  ) => ({
    choices,
    order: choices,
    index,
    next,
    shuffled: false,
    repeat,
    loaded: true,
  });

  it('joins the queue after the track loaded next', () => {
    // b plays and c is loaded, so d loads next.
    const next = appendedQueue(playing(1, 3), rest, new Set())!;
    expect(ids(next.choices)).toBe('abcdefg');
    expect(next.reset).toBe(false);
    expect(next.choices[next.next].id).toBe('d');
  });

  it('does not bring back a track removed while it was read', () => {
    const removed = removedQueue(playing(0, 1), 1);
    expect(ids(removed.choices)).toBe('ac');
    const next = appendedQueue(
      { ...playing(removed.index, 1), ...removed },
      rest,
      new Set(),
    )!;
    expect(ids(next.choices)).toBe('acdefg');
  });

  it('leaves a track added by hand where it was added', () => {
    const added = insertedQueue(
      { ...playing(0, 1), finished: false },
      list('e'),
      false,
    );
    expect(ids(added.choices)).toBe('abce');
    const next = appendedQueue(
      { ...playing(0, 1), ...added },
      rest,
      new Set(['e']),
    )!;
    expect(ids(next.choices)).toBe('abcedfg');
    expect(appendedQueue(playing(0, 1), list('e'), new Set(['e']))).toBe(null);
  });

  it('goes on instead of starting over when the last track repeats the queue', () => {
    // c plays last and the queue would start over with a.
    const next = appendedQueue(playing(2, 0, 'all'), rest, new Set())!;
    expect(next.reset).toBe(true);
    expect(playsAfter({ ...next, repeat: 'all' }, 2)).toBe(3);
  });

  it('keeps the track a repeating queue loaded next', () => {
    // b plays and c is loaded, after which the queue would start over.
    const next = appendedQueue(playing(1, 0, 'all'), rest, new Set())!;
    expect(next.reset).toBe(false);
    expect(next.choices[next.next].id).toBe('d');
  });
});
