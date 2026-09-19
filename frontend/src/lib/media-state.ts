import { api } from './api';
import type { MediaChoice } from './playback';
export type Card = MediaChoice & {
  kind: string;
  available: boolean;
  show_title?: string;
  edition?: string;
  position?: number;
  duration?: number;
};
export type SavedQueue = {
  revision: number;
  items: string[];
  current_index: number;
  position: number;
  tracks?: Card[];
  completed?: boolean;
};
export function clientId(): string {
  let id = localStorage.getItem('thelxinoe-client-id');
  if (!id || !/^[0-9a-f-]{36}$/.test(id)) {
    id = crypto.randomUUID();
    localStorage.setItem('thelxinoe-client-id', id);
  }
  return id;
}
export function choiceIndex(items: MediaChoice[], choice: MediaChoice): number {
  return Math.max(
    0,
    items.findIndex((item) =>
      choice.queue_context
        ? item.queue_context?.index === choice.queue_context.index
        : item.id === choice.id,
    ),
  );
}
export async function persistQueue(choice: MediaChoice): Promise<MediaChoice> {
  if (choice.kind !== 'track' || choice.queue_context) return choice;
  const client = clientId(),
    path = `/me/queue/${client}`;
  const previous = await api<SavedQueue>(path);
  const items = choice.queue?.length ? choice.queue : [choice];
  const index = choice.queueIndex ?? choiceIndex(items, choice);
  const saved = await api<SavedQueue>(path, 'PUT', {
    revision: previous.revision,
    items: items.map((i) => i.id),
    current_index: index,
  });
  const queue = items.map((item, index) => ({
    ...item,
    queue: undefined,
    kind: 'track',
    queue_context: { client_id: client, revision: saved.revision, index },
  }));
  return {
    ...choice,
    position: 0,
    queue_context: queue[index].queue_context,
    queue,
  };
}
export function restoreQueue(
  saved: SavedQueue,
  index = saved.current_index,
): MediaChoice | undefined {
  const client = clientId();
  const queue = saved.items
    .map((id, index) => {
      const track = saved.tracks?.find((t) => t.id === id);
      return track?.available
        ? {
            ...track,
            queue_context: {
              client_id: client,
              revision: saved.revision,
              index,
            },
          }
        : undefined;
    })
    .filter(
      (
        item,
      ): item is Card & {
        queue_context: { client_id: string; revision: number; index: number };
      } => !!item,
    );
  const chosen = queue.find((i) => i.queue_context.index >= index) ?? queue[0];
  if (saved.completed && queue.length) {
    const replay = queue.map((item) => ({ ...item, queue_context: undefined }));
    return { ...replay[0], position: 0, queue: replay };
  }
  return chosen
    ? {
        ...chosen,
        position:
          chosen.queue_context.index === saved.current_index
            ? saved.position
            : 0,
        queue,
      }
    : undefined;
}
