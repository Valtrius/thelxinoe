import type { MediaChoice } from '../playback';

function reshuffled<T>(items: T[]): T[] {
  const result = [...items];
  for (let i = result.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [result[i], result[j]] = [result[j], result[i]];
  }
  return result;
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
