import type { MediaChoice } from './playback';
export type Card = MediaChoice & {
  kind: string;
  available: boolean;
  artwork_url?: string;
  year?: number | null;
  show_title?: string;
  edition?: string;
  position?: number;
  duration?: number;
};
/** The choice's place in its queue: its `queueIndex`, or else the first entry
 * for the same media. */
export function choiceIndex(items: MediaChoice[], choice: MediaChoice): number {
  return Math.max(
    0,
    items.findIndex((item, index) =>
      choice.queueIndex === undefined
        ? item.id === choice.id
        : index === choice.queueIndex,
    ),
  );
}
/** A track plays from the start with its queue, or a queue of itself. Each
 * entry carries its own place, so a track listed twice keeps its position. */
export function musicQueue(choice: MediaChoice): MediaChoice {
  if (choice.kind !== 'track') return choice;
  const items = choice.queue?.length ? choice.queue : [choice];
  const index = choiceIndex(items, choice);
  return {
    ...choice,
    position: 0,
    queueIndex: index,
    queue: items.map((item, queueIndex) => ({
      ...item,
      queue: undefined,
      kind: 'track',
      queueIndex,
    })),
  };
}
