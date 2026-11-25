import { beforeEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError } from '../api';
import type { MediaChoice } from '../playback';
import { restOf, type MusicItem } from './library';
import { Queue, savedChoice, type QueueChange, type Repeat } from './queue';

vi.mock('../api', async (actual) => ({
  ...(await actual<typeof import('../api')>()),
  api: vi.fn(),
}));
const read = vi.mocked(api);

const list = (ids: string) =>
  [...ids].map((id): MediaChoice => ({ id, title: id, kind: 'track' }));
const ids = (choices: MediaChoice[]) => choices.map((c) => c.id).join('');
const backwards = <T>(items: T[]) => [...items].reverse();
const track = (id: string): MusicItem => ({
  id,
  kind: 'track',
  title: id,
  year: null,
  available: true,
  metadata: {},
});
/** The followed Playlist now holds `tracks`, at `revision`. */
function playlist(tracks: string, revision: number) {
  read.mockResolvedValueOnce({
    id: 'list',
    revision,
    items: [...tracks].map(track),
    sort_mode: 'manual',
    sort_direction: 'asc',
  });
}
const settle = () => new Promise((resolve) => setTimeout(resolve));

/**
 * The audio engine's side, without audio: it holds the playing track and the
 * one read after it, and follows the Queue's changes as the engine does.
 */
class Engine {
  held: { at: number; choice: MediaChoice }[] = [];
  reading = false;
  constructor(readonly queue: Queue) {
    queue.attach({
      holding: () => ({ held: this.held.length, reading: this.reading }),
      changed: (change) => this.changed(change),
    });
    this.fill();
  }
  private changed(change: QueueChange) {
    if (change === 'playing') {
      this.held = [];
      return this.fill();
    }
    if (change === 'ahead') this.held = this.held.slice(0, 1);
    if (change !== 'kept') {
      let at = this.queue.index;
      for (const entry of this.held) {
        entry.at = at;
        at = this.queue.after(at);
      }
    }
    this.fill();
  }
  private fill() {
    while (this.held.length < 2 && this.queue.unread)
      this.held.push(this.queue.read()!);
  }
  get playing() {
    return this.queue.current?.id ?? '';
  }
  get upNext() {
    return this.held[1]?.choice.id ?? '';
  }
  /** The playing track ends. */
  end() {
    const done = this.held.shift()!;
    if (
      !this.queue.reached(
        this.held[0]?.at ?? Math.min(done.at + 1, this.queue.length),
      )
    )
      this.fill();
  }
  /** What plays from now on, until the Queue finishes or `limit` tracks. */
  playOut(limit = 20) {
    const played: string[] = [];
    while (this.playing && played.length < limit) {
      played.push(this.playing);
      this.end();
    }
    return played.join('');
  }
  jump(at: number) {
    this.held = [];
    this.queue.readFrom(at);
    this.fill();
  }
  skip() {
    const target = this.queue.nextTrack();
    if (target >= this.queue.length) {
      this.held = [];
      return this.queue.readFrom(this.queue.length);
    }
    if (this.held[1]?.at !== target) return this.jump(target);
    this.held.shift();
    this.queue.moveTo(target);
    this.fill();
  }
  /** Previous, `position` seconds into the playing track. */
  previous(position: number) {
    const earlier = this.queue.previous(position);
    if (earlier !== null) this.jump(earlier);
  }
}

function start(
  tracks: string,
  options: {
    at?: number;
    repeat?: Repeat;
    shuffle?: boolean;
    following?: boolean;
    more?: string;
  } = {},
) {
  const queue = new Queue(list(tracks), {
    start: options.at,
    playlist: options.following ? { id: 'list', revision: 1 } : undefined,
    more: options.more
      ? async () => restOf([...options.more!].map(track), list(tracks))
      : undefined,
    shuffle: backwards,
  });
  if (options.repeat) queue.setRepeat(options.repeat);
  if (options.shuffle) queue.setShuffle(true);
  return new Engine(queue);
}

beforeEach(() => read.mockReset());

describe('a Queue following its Playlist', () => {
  it('drops a track added by hand when the Playlist changes, then unshuffles to the Playlist order', async () => {
    const engine = start('abcd', { following: true, shuffle: true });
    const { queue } = engine;
    expect(ids(queue.choices)).toBe('adcb');
    queue.insert(list('x'), true);
    expect(engine.upNext).toBe('x');
    playlist('abce', 2);
    queue.playlistChanged('list');
    await settle();
    expect(read).toHaveBeenCalledWith('/playlists/list');
    // The Playlist wins over hand edits: x and d left, e came, the rest is drawn again.
    expect(ids(queue.choices)).toBe('aecb');
    expect(engine.upNext).toBe('e');
    queue.setShuffle(false);
    expect(ids(queue.choices)).toBe('abce');
    expect(engine.upNext).toBe('b');
    expect(engine.playOut()).toBe('abce');
  });

  it('ignores other Playlists and revisions it already has', async () => {
    const engine = start('abc', { following: true });
    engine.queue.playlistChanged('other');
    expect(read).not.toHaveBeenCalled();
    playlist('cba', 1);
    engine.queue.playlistChanged('list');
    await settle();
    expect(ids(engine.queue.choices)).toBe('abc');
  });

  it('stops following once cleared', async () => {
    const engine = start('abcd', { following: true, at: 1 });
    engine.queue.clear();
    expect(ids(engine.queue.choices)).toBe('b');
    engine.queue.playlistChanged('list');
    await engine.queue.syncPlaylist();
    expect(read).not.toHaveBeenCalled();
    expect(engine.queue.save(0, 0).playlist).toBeNull();
    expect(engine.playOut()).toBe('b');
  });

  it('stops following a Playlist that was deleted', async () => {
    const engine = start('abc', { following: true });
    read.mockRejectedValueOnce(new ApiError(404, 'not_found', 'Not found'));
    await engine.queue.syncPlaylist();
    engine.queue.playlistChanged('list');
    expect(read).toHaveBeenCalledTimes(1);
    expect(engine.queue.save(0, 0).playlist).toBeNull();
  });

  it('keeps the track loaded next when it still comes next', async () => {
    const engine = start('abcd', { following: true });
    const loaded = engine.held[1];
    playlist('abdc', 2);
    await engine.queue.syncPlaylist();
    expect(engine.held[1]).toBe(loaded);
    expect(engine.playOut()).toBe('abdc');
  });

  // From the Queue playing `from` at `at`, after the Playlist becomes `to`.
  it.each([
    ['moves the playing track to its new place', 'abcd', 1, 'cbad', 'cbad', 1],
    [
      'drops hand-added tracks and restores hand-removed ones',
      'axc',
      0,
      'abc',
      'abc',
      0,
    ],
    [
      'lets a removed playing track finish, then what followed it',
      'abcd',
      1,
      'acd',
      'abcd',
      1,
    ],
    [
      'skips followers that left with the playing track',
      'abcd',
      1,
      'ad',
      'abd',
      1,
    ],
    [
      'plays a hand-removed track that comes back after a removed playing one',
      'abd',
      1,
      'acd',
      'abcd',
      1,
    ],
    [
      'plays what was added after a removed last track',
      'abc',
      2,
      'abd',
      'abcd',
      2,
    ],
    ['keeps a removed last track at the end', 'abc', 2, 'ab', 'abc', 2],
  ])('%s', async (_, from, at, to, choices, index) => {
    const engine = start(from, { following: true, at });
    playlist(to, 2);
    await engine.queue.syncPlaylist();
    expect(ids(engine.queue.choices)).toBe(choices);
    expect(engine.queue.index).toBe(index);
  });

  it('plays a removed playing track once, even when repeating it', async () => {
    const engine = start('abc', { following: true, at: 1, repeat: 'one' });
    playlist('ac', 2);
    await engine.queue.syncPlaylist();
    expect(engine.queue.leaving).toBe(true);
    expect(engine.upNext).toBe('c');
    engine.end();
    expect(ids(engine.queue.choices)).toBe('ac');
    expect(engine.queue.leaving).toBe(false);
    expect(engine.playOut(3)).toBe('ccc');
  });

  it('keeps a shuffled Queue that was only reordered', async () => {
    const engine = start('abcd', { following: true, shuffle: true });
    const before = engine.queue.choices;
    playlist('dcba', 2);
    await engine.queue.syncPlaylist();
    expect(engine.queue.choices).toBe(before);
    engine.queue.setShuffle(false);
    expect(ids(engine.queue.choices)).toBe('dcba');
  });

  it('forgets played tracks that left a shuffled Queue', async () => {
    const engine = start('abcd', { following: true, shuffle: true });
    engine.end();
    engine.end();
    expect(engine.playing).toBe('c');
    playlist('abc', 2);
    await engine.queue.syncPlaylist();
    expect(ids(engine.queue.choices)).toBe('acb');
    expect(engine.queue.index).toBe(1);
  });
});

describe('Previous', () => {
  it.each([
    ['off', 1, 'a', 'b'],
    ['one', 1, 'a', 'a'],
    ['all', 1, 'c', 'a'],
    ['all', 5, 'a', 'b'],
  ] as const)(
    'at the start, repeating %s, %s s in, plays %s then %s',
    (repeat, position, plays, then) => {
      const engine = start('abc', { repeat });
      engine.previous(position);
      expect(engine.playing).toBe(plays);
      expect(engine.upNext).toBe(then);
    },
  );

  it('plays the track before a few seconds in', () => {
    const engine = start('abc', { at: 2 });
    engine.previous(2);
    expect(engine.playing).toBe('b');
    engine.previous(4);
    expect(engine.playing).toBe('b');
  });
});

describe('Next', () => {
  it('is not held back by repeating one track', () => {
    const engine = start('abc', { repeat: 'one' });
    expect(engine.upNext).toBe('a');
    engine.skip();
    expect(engine.playing).toBe('b');
  });

  it('starts over or finishes after the last track', () => {
    const repeating = start('ab', { at: 1, repeat: 'all' });
    repeating.skip();
    expect(repeating.playing).toBe('a');
    const finishing = start('ab', { at: 1 });
    finishing.skip();
    expect(finishing.playing).toBe('');
  });
});

describe('the rest of the list', () => {
  it('mixes into a shuffled Queue after the track loaded next', async () => {
    const engine = start('abc', { shuffle: true, more: 'abcdefg' });
    expect(ids(engine.queue.choices)).toBe('acb');
    expect(engine.upNext).toBe('c');
    engine.queue.extend();
    await settle();
    expect(ids(engine.queue.choices)).toBe('acgfedb');
    expect(engine.playOut()).toBe('acgfedb');
    engine.queue.setShuffle(false);
    expect(ids(engine.queue.choices)).toBe('abcdefg');
  });

  it('leaves a track added by hand where it was added', async () => {
    const engine = start('abc', { more: 'abcdef' });
    engine.queue.insert(list('e'), false);
    engine.queue.extend();
    await settle();
    expect(ids(engine.queue.choices)).toBe('abcedf');
  });

  it('does not bring back a track removed while it was read', async () => {
    const engine = start('abc', { more: 'abcde' });
    engine.queue.remove(1);
    expect(engine.upNext).toBe('c');
    engine.queue.extend();
    await settle();
    expect(engine.playOut()).toBe('acde');
  });

  it('goes on instead of starting over when the last track repeats the Queue', async () => {
    const engine = start('abc', { at: 2, repeat: 'all', more: 'abcde' });
    expect(engine.upNext).toBe('a');
    engine.queue.extend();
    await settle();
    expect(engine.upNext).toBe('d');
    expect(engine.playOut(6)).toBe('cdeabc');
  });

  it('keeps the track a repeating Queue loaded next', async () => {
    const engine = start('abc', { at: 1, repeat: 'all', more: 'abcde' });
    const loaded = engine.held[1];
    engine.queue.extend();
    await settle();
    expect(engine.held[1]).toBe(loaded);
    expect(engine.playOut(6)).toBe('bcdeab');
  });

  it('is not added once the Queue was cleared', async () => {
    const engine = start('abc', { more: 'abcde' });
    engine.queue.clear();
    engine.queue.extend();
    await settle();
    expect(ids(engine.queue.choices)).toBe('a');
  });
});

describe('a finished Queue', () => {
  it('plays a track added to it', () => {
    const engine = start('ab', { at: 1 });
    engine.end();
    expect(engine.playing).toBe('');
    engine.queue.insert(list('x'), true);
    expect(engine.playing).toBe('x');
    expect(engine.playOut()).toBe('x');
  });

  it('finishes when its playing track is removed last', () => {
    const engine = start('ab', { at: 1 });
    engine.queue.remove(1);
    expect(engine.playing).toBe('');
    expect(engine.queue.index).toBe(1);
  });
});

describe('saving the Queue', () => {
  const roundTrip = (value: unknown) => JSON.parse(JSON.stringify(value));

  it('opens again as it was left', async () => {
    const engine = start('abcd', {
      following: true,
      repeat: 'all',
      shuffle: true,
      at: 1,
    });
    playlist('bcd', 2);
    engine.queue.insert(list('x'), false);
    await engine.queue.syncPlaylist();
    const saved = engine.queue.save(42, 180);
    expect(saved).toMatchObject({
      index: 0,
      position: 42,
      duration: 180,
      repeat: 'all',
      shuffle: true,
      leaving: false,
      playlist: { id: 'list', revision: 2 },
    });

    const choice = savedChoice(roundTrip(saved))!;
    expect(choice).toMatchObject({ id: 'b', position: 42, duration: 180 });
    const restored = Queue.from(choice);
    expect(restored.restored).toBe(true);
    const again = restored.save(42, 180);
    expect(ids(again.queue)).toBe(ids(saved.queue));
    expect({ ...again, queue: [] }).toEqual({ ...saved, queue: [] });
    expect(restored.current?.position).toBe(42);
    const engineAgain = new Engine(restored);
    expect(engineAgain.playOut(5)).toBe(engine.playOut(5));
  });

  it('keeps a leaving track leaving', async () => {
    const engine = start('abc', { following: true, at: 1 });
    playlist('ac', 2);
    await engine.queue.syncPlaylist();
    const restored = Queue.from(
      savedChoice(roundTrip(engine.queue.save(3, 10)))!,
    );
    expect(restored.leaving).toBe(true);
    expect(new Engine(restored).playOut()).toBe('bc');
  });

  it('catches up with its Playlist once played again', async () => {
    const engine = start('abc', { following: true });
    const restored = Queue.from(
      savedChoice(roundTrip(engine.queue.save(0, 0)))!,
    );
    playlist('acb', 2);
    await restored.syncPlaylist();
    expect(ids(restored.choices)).toBe('acb');
  });

  it('reads saves made before Playlists were followed', async () => {
    const choice = savedChoice({
      queue: list('abc'),
      index: 1,
      position: 30,
      duration: 0,
      repeat: 'one',
      shuffle: false,
    })!;
    expect(choice).toMatchObject({ id: 'b', position: 30 });
    const restored = Queue.from(choice);
    expect(restored.repeat).toBe('one');
    expect(restored.leaving).toBe(false);
    await restored.syncPlaylist();
    expect(read).not.toHaveBeenCalled();
    expect(restored.save(30, 0).playlist).toBeNull();
  });

  it('has nothing to open past the end', () => {
    expect(
      savedChoice({
        queue: list('ab'),
        index: 2,
        position: 0,
        duration: 0,
        repeat: 'off',
        shuffle: false,
      }),
    ).toBeNull();
    expect(savedChoice(null)).toBeNull();
  });
});
