import type { YoutubeWatchlist } from './types';

export function sortedYoutubeWatchlistItems(watchlist: YoutubeWatchlist) {
  const items = [...watchlist.items];
  if (watchlist.sortMode === 'manual') return items;
  return items.sort((left, right) => {
    if (left.metadataPending || right.metadataPending) {
      return (
        Number(Boolean(left.metadataPending)) -
          Number(Boolean(right.metadataPending)) ||
        left.manualPosition - right.manualPosition
      );
    }
    const delta =
      Date.parse(left.video.publishedAt) - Date.parse(right.video.publishedAt);
    return watchlist.sortDirection === 'asc' ? delta : -delta;
  });
}

export function watchlistVideoTransitionedToWatched(
  watchlists: YoutubeWatchlist[],
  videoId: string,
  nextIsWatched: boolean,
): boolean {
  if (!nextIsWatched) return false;
  return watchlists.some(
    (watchlist) =>
      watchlist.autoRemoveWatched &&
      watchlist.items.some(
        (item) => item.video.videoId === videoId && !item.video.isWatched,
      ),
  );
}

export class KeyedAsyncQueue {
  private tails = new Map<string, Promise<void>>();

  run<T>(key: string, operation: () => Promise<T>): Promise<T> {
    const previous = this.tails.get(key) ?? Promise.resolve();
    const result = previous.catch(() => undefined).then(operation);
    const tail = result.then(
      () => undefined,
      () => undefined,
    );
    this.tails.set(key, tail);
    void tail.then(() => {
      if (this.tails.get(key) === tail) this.tails.delete(key);
    });
    return result;
  }
}
