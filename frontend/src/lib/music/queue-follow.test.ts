import { describe, expect, it } from 'vitest';
import type { MediaChoice } from '../playback';
import { followedQueue } from './queue-follow';

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
