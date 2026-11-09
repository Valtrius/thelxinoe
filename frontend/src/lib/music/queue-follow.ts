import type { Repeat } from '../gapless';
import type { MediaChoice } from '../playback';

function reshuffled<T>(items: T[]): T[] {
  const result = [...items];
  for (let i = result.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [result[i], result[j]] = [result[j], result[i]];
  }
  return result;
}

type Upcoming = {
  choices: MediaChoice[];
  repeat: Repeat;
  /** The playing choice, when its playlist lost it: it goes as it stops. */
  leaving?: MediaChoice;
};

/** Whether the queue starts over after its end. */
export function wraps(queue: Upcoming): boolean {
  return (
    queue.repeat !== 'off' &&
    queue.choices.some((choice) => choice !== queue.leaving)
  );
}

/** Where what plays after the choice at `at` is, or past the end. */
export function playsAfter(queue: Upcoming, at: number): number {
  if (queue.repeat === 'one' && queue.choices[at] !== queue.leaving) return at;
  if (at + 1 < queue.choices.length) return at + 1;
  return wraps(queue) ? 0 : queue.choices.length;
}

/** A queue with tracks added after the playing one, or at the end. */
export function insertedQueue(
  queue: {
    choices: MediaChoice[];
    order: MediaChoice[];
    index: number;
    shuffled: boolean;
    /** Nothing plays or comes next, so the tracks play from where they land. */
    finished: boolean;
  },
  items: MediaChoice[],
  next: boolean,
): { choices: MediaChoice[]; order: MediaChoice[]; at: number } {
  const at =
    next && !queue.finished
      ? Math.min(queue.index + 1, queue.choices.length)
      : queue.choices.length;
  const choices = [
    ...queue.choices.slice(0, at),
    ...items,
    ...queue.choices.slice(at),
  ];
  return {
    choices,
    order: queue.shuffled ? [...queue.order, ...items] : [...choices],
    at,
  };
}

/** A queue without the choice at `at`; the playing one keeps its place. */
export function removedQueue(
  queue: { choices: MediaChoice[]; order: MediaChoice[]; index: number },
  at: number,
): { choices: MediaChoice[]; order: MediaChoice[]; index: number } {
  const removed = queue.choices[at];
  return {
    choices: queue.choices.filter((_, i) => i !== at),
    order: queue.order.filter((choice) => choice !== removed),
    index: at < queue.index ? queue.index - 1 : queue.index,
  };
}

/**
 * A queue once the rest of its list arrived, without the tracks added by hand
 * meanwhile. It asks to `reset` what was loaded ahead when that no longer
 * comes next; otherwise `next` is where loading carries on.
 */
export function appendedQueue(
  queue: Upcoming & {
    order: MediaChoice[];
    index: number;
    next: number;
    shuffled: boolean;
    /** A track plays or is being read. */
    loaded: boolean;
  },
  list: MediaChoice[],
  inserted: ReadonlySet<string>,
  shuffle: <T>(items: T[]) => T[] = reshuffled,
): {
  choices: MediaChoice[];
  order: MediaChoice[];
  next: number;
  reset: boolean;
} | null {
  const items = list.filter((item) => !inserted.has(item.id));
  if (!items.length) return null;
  const upNext = queue.choices[playsAfter(queue, queue.index)];
  const { choices, order } = extendedQueue(queue, items, shuffle);
  // A track at the end of the queue now has the new tracks after it.
  if (choices[playsAfter({ ...queue, choices }, queue.index)] !== upNext)
    return { choices, order, next: queue.next, reset: true };
  // Past the tracks loaded, a repeating queue no longer starts over.
  const next =
    queue.repeat === 'all' && queue.next === 0 && queue.loaded
      ? queue.choices.length
      : queue.next;
  return { choices, order, next, reset: false };
}

/**
 * A queue with the rest of its list. Shuffled, the new tracks mix into what is
 * still to come, after the next track, which may be loaded already.
 */
export function extendedQueue(
  queue: {
    choices: MediaChoice[];
    order: MediaChoice[];
    index: number;
    shuffled: boolean;
  },
  items: MediaChoice[],
  shuffle: <T>(items: T[]) => T[] = reshuffled,
): { choices: MediaChoice[]; order: MediaChoice[] } {
  const loaded = queue.index + 2;
  return {
    choices: queue.shuffled
      ? [
          ...queue.choices.slice(0, loaded),
          ...shuffle([...queue.choices.slice(loaded), ...items]),
        ]
      : [...queue.choices, ...items],
    order: [...queue.order, ...items],
  };
}

/**
 * A queue after its playlist changed: the playlist wins over hand edits. The
 * playing track stays, even when removed, so it finishes and what followed it
 * comes next. A shuffled queue keeps what was played, and only draws the rest
 * again when the tracks themselves changed.
 */
export function followedQueue(
  queue: { choices: MediaChoice[]; index: number; shuffled: boolean },
  items: MediaChoice[],
  shuffle: <T>(items: T[]) => T[] = reshuffled,
): { choices: MediaChoice[]; order: MediaChoice[]; index: number } {
  const current = queue.choices[queue.index];
  const known = new Map(queue.choices.map((choice) => [choice.id, choice]));
  known.set(current.id, current);
  const listed = new Set(items.map((item) => item.id));
  let order = items.map((item) => known.get(item.id) ?? item);
  if (!listed.has(current.id)) {
    const follower = queue.choices
      .slice(queue.index + 1)
      .find((choice) => listed.has(choice.id));
    let at = follower ? order.indexOf(follower) : order.length;
    // What the queue didn't have comes after it, instead of being passed by.
    while (at > 0 && !known.has(order[at - 1].id)) at--;
    order = [...order.slice(0, at), current, ...order.slice(at)];
  }
  if (!queue.shuffled)
    return { choices: order, order, index: order.indexOf(current) };

  const present = new Set(order);
  if (
    present.size === queue.choices.length &&
    queue.choices.every((choice) => present.has(choice))
  )
    return { choices: queue.choices, order, index: queue.index };
  const played = queue.choices
    .slice(0, queue.index)
    .filter((choice) => present.has(choice));
  const behind = new Set([...played, current]);
  return {
    choices: [
      ...played,
      current,
      ...shuffle(order.filter((choice) => !behind.has(choice))),
    ],
    order,
    index: played.length,
  };
}
