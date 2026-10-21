import { SvelteDate, SvelteMap, SvelteSet } from 'svelte/reactivity';
import { api, normalizeError } from './api';
import { dismissToastByKey, showToast } from './toasts';
import { KeyedAsyncQueue } from './youtube-watchlists';
import type {
  AppError,
  YoutubeVideo,
  YoutubeWatchlist,
  YoutubeWatchlistItem,
  YoutubeWatchlistUpdate,
  YoutubeProgressEvent,
} from './types';

export const manualWatchFilterDelayMs = 5_000;
export function createYoutubeWatchlists(savedSelectedWatchlistId: number) {
  let error = $state<AppError | null>(null);
  let watchlists = $state<YoutubeWatchlist[]>([]);
  let selectedWatchlistId = $state<number | null>(
    Number.isSafeInteger(savedSelectedWatchlistId) &&
      savedSelectedWatchlistId > 0
      ? savedSelectedWatchlistId
      : null,
  );
  const downloadBusyIds = new SvelteSet<string>();
  const pendingManualWatchedVideoIds = new SvelteSet<string>();
  const watchlistMembershipIntent = new SvelteMap<string, boolean>();
  const watchlistMembershipQueue = new KeyedAsyncQueue();
  const localAdditions = new SvelteMap<
    string,
    {
      watchlistId: number;
      item: YoutubeWatchlistItem;
      confirmedRevision: number | null;
    }
  >();
  let additionRevision = 0;
  let orderRevision = 0;
  const localOrders = new SvelteMap<
    number,
    { ids: string[]; confirmedRevision: number | null }
  >();
  const reorderQueue = new KeyedAsyncQueue();
  const displayedWatchlists = $derived.by(() =>
    watchlists.map((watchlist) => {
      const items = [...watchlist.items];
      for (const addition of localAdditions.values()) {
        if (
          addition.watchlistId === watchlist.id &&
          !items.some(
            (item) => item.video.videoId === addition.item.video.videoId,
          )
        ) {
          items.push(addition.item);
        }
      }
      const order = localOrders.get(watchlist.id);
      const positions = new SvelteMap(
        order?.ids.map((id, index) => [id, index]),
      );
      items.sort(
        (left, right) =>
          (positions.get(left.video.videoId) ?? left.manualPosition) -
          (positions.get(right.video.videoId) ?? right.manualPosition),
      );
      if (order)
        items.forEach((item, index) => {
          items[index] = { ...item, manualPosition: index };
        });
      return { ...watchlist, items };
    }),
  );
  const watchlistUpdateVersions = new SvelteMap<number, number>();
  const watchlistUpdateQueues = new SvelteMap<number, Promise<void>>();
  let watchlistLoadSequence = 0;

  async function loadWatchlists(
    preferredId = selectedWatchlistId,
    reportError = true,
  ): Promise<boolean> {
    const selectionAtStart = selectedWatchlistId;
    const sequence = ++watchlistLoadSequence;
    const confirmedBeforeLoad = additionRevision;
    const ordersBeforeLoad = orderRevision;
    try {
      const next = await api.youtubeWatchlists();
      if (sequence !== watchlistLoadSequence) return false;
      for (const [id, order] of localOrders) {
        if (
          order.confirmedRevision !== null &&
          order.confirmedRevision <= ordersBeforeLoad
        )
          localOrders.delete(id);
      }
      // Only a refresh started after an add completed can confirm its membership.
      // Older responses still apply unrelated list changes beneath the local entries.
      for (const [key, addition] of localAdditions) {
        if (
          addition.confirmedRevision !== null &&
          addition.confirmedRevision <= confirmedBeforeLoad
        ) {
          localAdditions.delete(key);
        }
      }
      watchlists = next;
      const preferred =
        selectedWatchlistId === selectionAtStart
          ? preferredId
          : selectedWatchlistId;
      selectedWatchlistId =
        next.find((watchlist) => watchlist.id === preferred)?.id ??
        next[0]?.id ??
        null;
      return true;
    } catch (caught) {
      if (sequence === watchlistLoadSequence && reportError) {
        error = normalizeError(caught);
      }
      return false;
    }
  }

  function updateWatchlistVideo(
    videoId: string,
    update: (video: YoutubeVideo) => YoutubeVideo,
  ) {
    for (const addition of localAdditions.values()) {
      if (addition.item.video.videoId === videoId) {
        addition.item = {
          ...addition.item,
          video: update(addition.item.video),
        };
      }
    }
    watchlists = watchlists.map((watchlist) => ({
      ...watchlist,
      items: watchlist.items.map((item) =>
        item.video.videoId === videoId
          ? { ...item, video: update(item.video) }
          : item,
      ),
    }));
  }

  function membershipKey(watchlistId: number, videoId: string) {
    return `${watchlistId}:${videoId}`;
  }

  async function addToWatchlist(
    input: YoutubeVideo | string,
    watchlistId: number,
  ): Promise<boolean> {
    const video = typeof input === 'string' ? null : input;
    const videoId = typeof input === 'string' ? input : input.videoId;
    error = null;
    const key = membershipKey(watchlistId, videoId);
    watchlistMembershipIntent.set(key, true);
    const watchlist = displayedWatchlists.find(
      (item) => item.id === watchlistId,
    );
    const alreadyAdded = watchlist?.items.some(
      (item) => item.video.videoId === videoId,
    );
    if (alreadyAdded) return true;

    // Choose the saved position before metadata requests can complete out of order.
    const optimisticPosition = watchlist
      ? Math.max(-1, ...watchlist.items.map((item) => item.manualPosition)) + 1
      : 0;
    const optimisticItem: YoutubeWatchlistItem = {
      video: video ?? {
        videoId,
        channelId: '',
        channelName: '',
        title: 'Loading video…',
        publishedAt: '',
        isLive: false,
        isUpcoming: false,
        isLiveReplay: false,
        broadcastState: 'none',
        positionSeconds: 0,
        watchedPercentage: 0,
        isWatched: false,
      },
      addedAt: new SvelteDate().toISOString(),
      manualPosition: optimisticPosition,
      metadataPending: !video,
    };
    const addition = $state({
      watchlistId,
      item: optimisticItem,
      confirmedRevision: null as number | null,
    });
    localAdditions.set(key, addition);

    const autoDownloadStarting = Boolean(watchlist?.autoDownload);
    if (autoDownloadStarting) downloadBusyIds.add(videoId);

    try {
      const result = await watchlistMembershipQueue.run(key, () =>
        api.addYoutubeVideoToWatchlist(
          watchlistId,
          videoId,
          optimisticPosition,
        ),
      );
      if (localAdditions.get(key) !== addition) return false;
      if (watchlistMembershipIntent.get(key) === true) {
        addition.item = {
          ...addition.item,
          video: result.video,
          metadataPending: false,
        };
        addition.confirmedRevision = ++additionRevision;
      } else {
        localAdditions.delete(key);
      }
      if (result.autoDownloadError) {
        showToast({
          key: `watchlist-auto-download-${videoId}`,
          tone: 'warning',
          title: 'Added to watchlist, download not started',
          message: result.autoDownloadError.message,
        });
      } else {
        dismissToastByKey(`watchlist-auto-download-${videoId}`);
      }
      return true;
    } catch (caught) {
      if (localAdditions.get(key) !== addition) return false;
      localAdditions.delete(key);
      error = normalizeError(caught);
      return false;
    } finally {
      if (autoDownloadStarting) downloadBusyIds.delete(videoId);
    }
  }

  async function createWatchlist(name: string) {
    error = null;
    watchlistLoadSequence += 1;
    try {
      const id = await api.createYoutubeWatchlist(name);
      selectedWatchlistId = id;
      await loadWatchlists(id);
    } catch (caught) {
      error = normalizeError(caught);
    }
  }

  async function updateWatchlist(
    watchlistId: number,
    patch: Partial<YoutubeWatchlistUpdate>,
  ) {
    error = null;
    watchlistLoadSequence += 1;
    const current = watchlists.find(
      (watchlist) => watchlist.id === watchlistId,
    );
    if (!current) return;

    const update: YoutubeWatchlistUpdate = {
      name: current.name,
      autoDownload: current.autoDownload,
      autoRemoveWatched: current.autoRemoveWatched,
      sortMode: current.sortMode,
      sortDirection: current.sortDirection,
      ...patch,
    };
    watchlists = watchlists.map((watchlist) =>
      watchlist.id === watchlistId ? { ...watchlist, ...update } : watchlist,
    );

    const version = (watchlistUpdateVersions.get(watchlistId) ?? 0) + 1;
    watchlistUpdateVersions.set(watchlistId, version);
    const previous =
      watchlistUpdateQueues.get(watchlistId) ?? Promise.resolve();
    const operation = previous
      .catch(() => undefined)
      .then(async () => {
        try {
          await api.updateYoutubeWatchlist(watchlistId, update);
          if (watchlistUpdateVersions.get(watchlistId) === version) {
            await loadWatchlists(selectedWatchlistId);
          }
        } catch (caught) {
          if (watchlistUpdateVersions.get(watchlistId) === version) {
            const actionError = normalizeError(caught);
            await loadWatchlists(selectedWatchlistId, false);
            error = actionError;
          }
        }
      });
    watchlistUpdateQueues.set(watchlistId, operation);
    await operation;
    if (watchlistUpdateQueues.get(watchlistId) === operation) {
      watchlistUpdateQueues.delete(watchlistId);
    }
  }

  async function deleteWatchlist(watchlistId: number) {
    error = null;
    watchlistLoadSequence += 1;
    try {
      await api.deleteYoutubeWatchlist(watchlistId);
      await loadWatchlists();
    } catch (caught) {
      error = normalizeError(caught);
    }
  }

  async function removeFromWatchlist(watchlistId: number, video: YoutubeVideo) {
    error = null;
    watchlistLoadSequence += 1;
    const key = membershipKey(watchlistId, video.videoId);
    watchlistMembershipIntent.set(key, false);
    try {
      await watchlistMembershipQueue.run(key, () =>
        api.removeYoutubeVideoFromWatchlist(watchlistId, video.videoId),
      );
      localAdditions.delete(key);
      watchlists = watchlists.map((watchlist) =>
        watchlist.id === watchlistId
          ? {
              ...watchlist,
              items: watchlist.items.filter(
                (item) => item.video.videoId !== video.videoId,
              ),
            }
          : watchlist,
      );
    } catch (caught) {
      await loadWatchlists(selectedWatchlistId, false);
      error = normalizeError(caught);
    }
  }

  async function reorderWatchlist(watchlistId: number, videoIds: string[]) {
    error = null;
    watchlistLoadSequence += 1;
    const order = {
      ids: [...videoIds],
      confirmedRevision: null as number | null,
    };
    localOrders.set(watchlistId, order);
    try {
      await reorderQueue.run(String(watchlistId), () =>
        api.reorderYoutubeWatchlist(watchlistId, videoIds),
      );
      order.confirmedRevision = ++orderRevision;
      await loadWatchlists(watchlistId);
    } catch (caught) {
      if (localOrders.get(watchlistId) !== order) return;
      localOrders.delete(watchlistId);
      const actionError = normalizeError(caught);
      await loadWatchlists(watchlistId, false);
      error = actionError;
    }
  }

  // A watched video stays in an auto-remove watchlist, faded, until the server
  // removes it a few seconds later and its event reloads the lists.
  function applyProgress(progress: YoutubeProgressEvent): boolean {
    const manual = pendingManualWatchedVideoIds.delete(progress.videoId);
    updateWatchlistVideo(progress.videoId, (video) => ({
      ...video,
      positionSeconds: progress.positionSeconds,
      watchedPercentage: progress.watchedPercentage,
      isWatched: progress.isWatched,
    }));
    return manual;
  }
  function reset() {
    watchlistLoadSequence += 1;
    pendingManualWatchedVideoIds.clear();
    localAdditions.clear();
    localOrders.clear();
    watchlists = [];
    selectedWatchlistId = null;
  }
  return {
    get error() {
      return error;
    },
    get watchlists() {
      return displayedWatchlists;
    },
    get selectedWatchlistId() {
      return selectedWatchlistId;
    },
    set selectedWatchlistId(value: typeof selectedWatchlistId) {
      selectedWatchlistId = value;
    },
    get downloadBusyIds() {
      return downloadBusyIds;
    },
    get pendingManualWatchedVideoIds() {
      return pendingManualWatchedVideoIds;
    },
    loadWatchlists,
    updateWatchlistVideo,
    addToWatchlist,
    isAddingToWatchlist(watchlistId: number, videoId: string) {
      return (
        localAdditions.get(membershipKey(watchlistId, videoId))
          ?.confirmedRevision === null
      );
    },
    createWatchlist,
    updateWatchlist,
    deleteWatchlist,
    removeFromWatchlist,
    reorderWatchlist,
    applyProgress,
    reset,
  };
}
export type YoutubeWatchlistController = ReturnType<
  typeof createYoutubeWatchlists
>;
