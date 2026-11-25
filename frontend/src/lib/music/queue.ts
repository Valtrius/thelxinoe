import { api, ApiError } from '../api';
import { choiceIndex } from '../media-state';
import type { FollowedPlaylist, MediaChoice } from '../playback';
import { captureSession } from '../session';
import { playlistQueue, sortedPlaylistTracks } from './playlist-order';
import type { PlaylistDetail } from './playlists.svelte';

export type Repeat = 'off' | 'all' | 'one';
type Shuffle = <T>(items: T[]) => T[];

/** How a change of the Queue concerns the audio engine reading it. */
export type QueueChange =
  /** What the engine holds or reads still sits where it was. */
  | 'kept'
  /** What the engine holds still plays in that order, now from `index` on. */
  | 'moved'
  /** What the engine loaded ahead no longer comes next. */
  | 'ahead'
  /** The playing track left: `index` plays from its start, or nothing past the end. */
  | 'playing';

/** The audio engine's side of a Queue. */
export type QueueReader = {
  /** How many tracks it holds from the playing one on, and whether it reads one more. */
  holding(): { held: number; reading: boolean };
  changed(change: QueueChange): void;
};

/** A Queue as this device keeps it, to open it again as it was left. */
export type SavedQueue = {
  queue: MediaChoice[];
  index: number;
  position: number;
  duration: number;
  repeat: Repeat;
  shuffle: boolean;
  leaving: boolean;
  playlist: FollowedPlaylist | null;
};

function shuffled<T>(items: T[]): T[] {
  const result = [...items];
  for (let i = result.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [result[i], result[j]] = [result[j], result[i]];
  }
  return result;
}

/**
 * The tracks the Queue plays, in play order, and where it stands in them. A
 * Playlist it was started from keeps it in line as the Playlist changes; the
 * rest of a list it was started from joins it while it plays.
 */
export class Queue {
  #choices: MediaChoice[];
  /** The Queue before it was shuffled. */
  #order: MediaChoice[];
  #index: number;
  /** The place the engine reads next. */
  #next: number;
  #repeat: Repeat = 'off';
  #shuffled = false;
  /** The playing choice, when its Playlist lost it: it goes as it stops. */
  #leaving?: MediaChoice;
  /** Tracks added by hand, which the rest of the list doesn't add again. */
  #inserted = new Set<string>();
  #more?: MediaChoice['more'];
  /** Reading the rest of the list; clearing ends it. */
  #extension = new AbortController();
  #following: FollowedPlaylist | null;
  #followRequest = 0;
  #reader?: QueueReader;
  #closed = false;
  #ownsSession = captureSession();
  #shuffle: Shuffle;
  /** Opened as it was left. */
  readonly restored: boolean;

  constructor(
    choices: MediaChoice[],
    options: {
      start?: number;
      parked?: MediaChoice['parked'];
      playlist?: FollowedPlaylist;
      more?: MediaChoice['more'];
      shuffle?: Shuffle;
    } = {},
  ) {
    this.#choices = [...choices];
    this.#order = [...choices];
    this.#index = Math.min(
      Math.max(0, options.start ?? 0),
      Math.max(0, choices.length - 1),
    );
    this.#next = this.#index;
    this.#following = options.playlist ?? null;
    this.#more = options.more;
    this.#shuffle = options.shuffle ?? shuffled;
    const { parked } = options;
    this.restored = !!parked;
    if (parked) {
      this.#repeat = parked.repeat;
      this.#shuffled = parked.shuffled;
      if (parked.leaving) this.#leaving = this.#choices[this.#index];
    }
  }

  /** The Queue a choice starts: its list, or itself. */
  static from(selected: MediaChoice, shuffle: Shuffle = shuffled) {
    const choices = selected.queue ?? [selected];
    const index = choiceIndex(choices, selected);
    // The chosen track keeps the file and position it was chosen with.
    const list = choices.map((entry, i) =>
      i === index
        ? {
            ...entry,
            ...selected,
            queue: undefined,
            shuffle: undefined,
            parked: undefined,
            more: undefined,
          }
        : { ...entry, queue: undefined },
    );
    const queue = new Queue(list, {
      start: selected.shuffle ? Math.floor(Math.random() * list.length) : index,
      parked: selected.parked,
      playlist: selected.playlist,
      more: selected.more,
      shuffle,
    });
    if (selected.shuffle) queue.setShuffle(true);
    return queue;
  }

  /** In play order; a new array whenever it changes. */
  get choices(): MediaChoice[] {
    return this.#choices;
  }
  get length() {
    return this.#choices.length;
  }
  /** The playing choice's place, or `length` once the Queue has finished. */
  get index() {
    return this.#index;
  }
  get current(): MediaChoice | undefined {
    return this.#choices[this.#index];
  }
  get repeat() {
    return this.#repeat;
  }
  get shuffled() {
    return this.#shuffled;
  }
  get leaving() {
    return !!this.#leaving;
  }
  /** A track is left for the engine to read. */
  get unread() {
    return this.#next < this.#choices.length;
  }

  attach(reader: QueueReader) {
    this.#reader = reader;
  }
  close() {
    this.#closed = true;
    this.#extension.abort();
  }

  /** The Queue starts over after its end. */
  #wraps() {
    return (
      this.#repeat !== 'off' &&
      this.#choices.some((choice) => choice !== this.#leaving)
    );
  }
  /** Where what plays after the choice at `at` is, or `length`. */
  after(at: number) {
    const choices = this.#choices;
    if (this.#repeat === 'one' && choices[at] !== this.#leaving) return at;
    if (at + 1 < choices.length) return at + 1;
    return this.#wraps() ? 0 : choices.length;
  }
  /** The next track for the engine to read, which it then reads past. */
  read(): { at: number; choice: MediaChoice } | undefined {
    const at = this.#next;
    if (at >= this.#choices.length) return undefined;
    this.#next = this.after(at);
    return { at, choice: this.#choices[at] };
  }
  /** The track at `at`, read already, plays now. */
  moveTo(at: number) {
    this.#index = at;
  }
  /**
   * The engine plays on at `at`. A leaving track that stopped goes; then this
   * tells the engine and answers true.
   */
  reached(at: number) {
    this.#index = at;
    if (!this.#leaving || this.current === this.#leaving) return false;
    const leaving = this.#choices.indexOf(this.#leaving);
    this.#choices = this.#choices.filter((choice) => choice !== this.#leaving);
    this.#order = this.#order.filter((choice) => choice !== this.#leaving);
    this.#leaving = undefined;
    if (leaving >= 0 && leaving < this.#index) this.#index--;
    this.#resetAhead();
    return true;
  }
  /** Play the track at `at`, read from there again; past the end, nothing plays. */
  readFrom(at: number) {
    this.#index = at;
    this.#next = at;
  }
  /** Where Next goes: repeating one track doesn't hold it back. */
  nextTrack() {
    const { length } = this.#choices;
    if (this.#index + 1 < length) return this.#index + 1;
    return this.#wraps() ? 0 : length;
  }
  /**
   * Where Previous goes `position` seconds into the playing track: the track
   * before it, or null to start this one again.
   */
  previous(position: number): number | null {
    const earlier =
      this.#index > 0
        ? this.#index - 1
        : this.#repeat === 'all'
          ? this.#choices.length - 1
          : -1;
    return position > 3 || earlier < 0 ? null : earlier;
  }

  #holding() {
    return this.#reader?.holding() ?? { held: 0, reading: false };
  }
  #changed(change: QueueChange) {
    this.#reader?.changed(change);
  }
  /** What was loaded ahead is read again under the new order. */
  #resetAhead() {
    this.#next = this.#holding().held
      ? this.after(this.#index)
      : Math.min(this.#index, this.#choices.length);
    this.#changed('ahead');
  }

  /** Add tracks after the playing one, or at the end. */
  insert(items: MediaChoice[], next: boolean) {
    if (this.#closed || !items.length) return;
    for (const item of items) this.#inserted.add(item.id);
    // Nothing plays or comes next, so the tracks play from where they land.
    const finished =
      !this.#holding().held && this.#next >= this.#choices.length;
    const at =
      next && !finished
        ? Math.min(this.#index + 1, this.#choices.length)
        : this.#choices.length;
    this.#choices = [
      ...this.#choices.slice(0, at),
      ...items,
      ...this.#choices.slice(at),
    ];
    this.#order = this.#shuffled
      ? [...this.#order, ...items]
      : [...this.#choices];
    if (finished) this.#index = at;
    this.#resetAhead();
  }
  /** Remove the choice at `at`; the playing one keeps its place. */
  remove(at: number) {
    if (this.#closed || at < 0 || at >= this.#choices.length) return;
    const removed = this.#choices[at];
    this.#choices = this.#choices.filter((_, i) => i !== at);
    this.#order = this.#order.filter((choice) => choice !== removed);
    if (at === this.#index) {
      this.readFrom(Math.min(at, this.#choices.length));
      this.#changed('playing');
      return;
    }
    if (at < this.#index) this.#index--;
    this.#resetAhead();
  }
  /** Keep only the playing track; the Queue no longer follows its Playlist. */
  clear() {
    this.#following = null;
    this.#followRequest++;
    if (this.#closed) return;
    this.#extension.abort();
    const current = this.current;
    this.#choices = current ? [current] : [];
    this.#order = [...this.#choices];
    this.#index = 0;
    this.#leaving = undefined;
    this.#resetAhead();
  }
  setShuffle(on: boolean) {
    if (this.#closed || on === this.#shuffled) return;
    const current = this.current;
    if (on) {
      this.#order = [...this.#choices];
      const rest = this.#shuffle(
        this.#choices.filter((_, i) => i !== this.#index),
      );
      this.#choices = current ? [current, ...rest] : rest;
      this.#index = 0;
    } else {
      this.#choices = [...this.#order];
      this.#index = Math.max(0, this.#choices.indexOf(current!));
    }
    this.#shuffled = on;
    this.#resetAhead();
  }
  setRepeat(mode: Repeat) {
    if (this.#closed || mode === this.#repeat) return;
    this.#repeat = mode;
    this.#resetAhead();
  }

  /** Read the rest of the list the Queue started from, and add it when it arrives. */
  extend() {
    const read = this.#more;
    const { signal } = this.#extension;
    if (!read || signal.aborted) return;
    // A Queue that can't get the rest plays on with what it has.
    read(signal).then(
      (items) => {
        if (!signal.aborted) this.#append(items);
      },
      () => {},
    );
  }
  /** The followed Playlist changed on the server. */
  playlistChanged(id: string | undefined) {
    if (this.#following && id === this.#following.id) void this.syncPlaylist();
  }
  /** Take the followed Playlist's latest order, if it changed. */
  async syncPlaylist() {
    const followed = this.#following;
    if (!followed) return;
    const request = ++this.#followRequest;
    const session = captureSession();
    try {
      const list = await api<PlaylistDetail>(`/playlists/${followed.id}`);
      if (request !== this.#followRequest || !session()) return;
      if (list.revision === followed.revision) return;
      this.#following = { id: list.id, revision: list.revision };
      const sorted = sortedPlaylistTracks(
        list.items,
        list.sort_mode,
        list.sort_direction,
      );
      this.#follow(playlistQueue(list, sorted)?.queue ?? []);
    } catch (e) {
      // Deleted or made private: the Queue carries on by itself.
      if (
        request === this.#followRequest &&
        e instanceof ApiError &&
        [403, 404].includes(e.status)
      )
        this.#following = null;
    }
  }
  /**
   * Take the Playlist's order: it wins over hand edits. The playing track
   * stays, even when removed, so it finishes and what followed it comes next.
   * A shuffled Queue keeps what was played, and only draws the rest again
   * when the tracks themselves changed. What is loaded ahead keeps playing
   * when it still comes next.
   */
  #follow(items: MediaChoice[]) {
    if (this.#closed || this.#index >= this.#choices.length) return;
    const upNext = this.#choices[this.after(this.#index)];
    const current = this.current!;
    const known = new Map(this.#choices.map((choice) => [choice.id, choice]));
    known.set(current.id, current);
    const listed = new Set(items.map((item) => item.id));
    let order = items.map((item) => known.get(item.id) ?? item);
    if (!listed.has(current.id)) {
      const follower = this.#choices
        .slice(this.#index + 1)
        .find((choice) => listed.has(choice.id));
      let at = follower ? order.indexOf(follower) : order.length;
      // What the Queue didn't have comes after it, instead of being passed by.
      while (at > 0 && !known.has(order[at - 1].id)) at--;
      order = [...order.slice(0, at), current, ...order.slice(at)];
    }
    this.#order = order;
    if (!this.#shuffled) {
      this.#choices = order;
      this.#index = order.indexOf(current);
    } else {
      const present = new Set(order);
      const same =
        present.size === this.#choices.length &&
        this.#choices.every((choice) => present.has(choice));
      if (!same) {
        const played = this.#choices
          .slice(0, this.#index)
          .filter((choice) => present.has(choice));
        const behind = new Set([...played, current]);
        this.#choices = [
          ...played,
          current,
          ...this.#shuffle(order.filter((choice) => !behind.has(choice))),
        ];
        this.#index = played.length;
      }
    }
    this.#leaving = listed.has(current.id) ? undefined : current;
    const { held, reading } = this.#holding();
    if (reading || !held || upNext !== this.#choices[this.after(this.#index)])
      return this.#resetAhead();
    this.#next = this.after(this.#index);
    if (held > 1) this.#next = this.after(this.#next);
    this.#changed('moved');
  }
  /**
   * The rest of the list arrived, without the tracks added by hand meanwhile.
   * Shuffled, it mixes into what is still to come, after the next track, which
   * may be loaded already.
   */
  #append(list: MediaChoice[]) {
    if (this.#closed || !this.#ownsSession()) return;
    const items = list.filter((item) => !this.#inserted.has(item.id));
    if (!items.length) return;
    const upNext = this.#choices[this.after(this.#index)];
    const loaded = this.#index + 2;
    const before = this.#choices.length;
    this.#choices = this.#shuffled
      ? [
          ...this.#choices.slice(0, loaded),
          ...this.#shuffle([...this.#choices.slice(loaded), ...items]),
        ]
      : [...this.#choices, ...items];
    this.#order = [...this.#order, ...items];
    // A track at the end of the Queue now has the new tracks after it.
    if (this.#choices[this.after(this.#index)] !== upNext)
      return this.#resetAhead();
    // Past the tracks loaded, a repeating Queue no longer starts over.
    const { held, reading } = this.#holding();
    if (this.#repeat === 'all' && this.#next === 0 && (reading || held > 0))
      this.#next = before;
    this.#changed('kept');
  }

  /** What to keep, `position` seconds into the playing track. */
  save(position: number, duration: number): SavedQueue {
    return {
      queue: this.#choices,
      index: this.#index,
      position,
      duration,
      repeat: this.#repeat,
      shuffle: this.#shuffled,
      leaving: !!this.#leaving,
      playlist: this.#following,
    };
  }
}

/**
 * The choice that opens a saved Queue as it was left: nothing is read until it
 * plays. Saves made before Playlists were followed have no `leaving` or `playlist`.
 */
export function savedChoice(
  saved:
    | (Omit<SavedQueue, 'leaving' | 'playlist'> &
        Partial<Pick<SavedQueue, 'leaving' | 'playlist'>>)
    | null,
): MediaChoice | null {
  const track = saved?.queue?.[saved.index];
  if (!saved || !track) return null;
  return {
    ...track,
    position: saved.position,
    duration: saved.duration || track.duration,
    queueIndex: saved.index,
    queue: saved.queue,
    parked: {
      repeat: saved.repeat,
      shuffled: saved.shuffle,
      leaving: saved.leaving,
    },
    playlist: saved.playlist ?? undefined,
  };
}
