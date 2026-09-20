import { SvelteDate, SvelteMap, SvelteSet } from 'svelte/reactivity';
import { api, normalizeError } from './api';
import { dismissToastByKey, showToast } from './toasts';
import {
  KeyedAsyncQueue,
  watchlistVideoTransitionedToWatched,
} from './youtube-watchlists';
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
  const deferredWatchlistRemovals = new SvelteMap<
    string,
    { timer: number; deadline: number; version: number }
  >();
  const autoRemoveVersions = new SvelteMap<string, number>();
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
      items.sort((left, right) => left.manualPosition - right.manualPosition);
      return { ...watchlist, items };
    }),
  );
  const watchlistUpdateVersions = new SvelteMap<number, number>();
  const watchlistUpdateQueues = new SvelteMap<number, Promise<void>>();
  let watchlistLoadSequence = 0;
  const activeRemovals = new SvelteSet<Promise<boolean>>();

  function trackRemoval(videoId: string, version: number) {
    const operation = removeFromAutoRemoveWatchlists(videoId, version);
    activeRemovals.add(operation);
    void operation.finally(() => activeRemovals.delete(operation));
    return operation;
  }
  function takeDeferredWatchlistRemovals(): Array<{
    videoId: string;
    remainingDelay: number;
    version: number;
  }> {
    const removals = [...deferredWatchlistRemovals].map(
      ([videoId, removal]) => ({
        videoId,
        remainingDelay: Math.max(0, removal.deadline - Date.now()),
        version: removal.version,
      }),
    );
    for (const removal of deferredWatchlistRemovals.values()) {
      window.clearTimeout(removal.timer);
    }
    deferredWatchlistRemovals.clear();
    return removals;
  }

  async function flushWatchlistRemovals(): Promise<boolean> {
    const removals = takeDeferredWatchlistRemovals();
    const results = await Promise.allSettled([
      ...activeRemovals,
      ...removals.map(({ videoId, version }) => trackRemoval(videoId, version)),
    ]);
    const saved = results.every(
      (result) => result.status === 'fulfilled' && result.value,
    );
    if (!saved) {
      for (const { videoId, remainingDelay } of removals) {
        if (hasAutoRemoveWatchlistItem(videoId)) {
          scheduleDeferredWatchlistRemoval(videoId, remainingDelay);
        }
      }
    }
    return saved;
  }

  async function loadWatchlists(
    preferredId = selectedWatchlistId,
    reportError = true,
  ): Promise<boolean> {
    const selectionAtStart = selectedWatchlistId;
    const sequence = ++watchlistLoadSequence;
    const confirmedBeforeLoad = additionRevision;
    try {
      const next = await api.youtubeWatchlists();
      if (sequence !== watchlistLoadSequence) return false;
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

  function hasAutoRemoveWatchlistItem(videoId: string) {
    return displayedWatchlists.some(
      (watchlist) =>
        watchlist.autoRemoveWatched &&
        watchlist.items.some((item) => item.video.videoId === videoId),
    );
  }

  function membershipKey(watchlistId: number, videoId: string) {
    return `${watchlistId}:${videoId}`;
  }

  function nextAutoRemoveVersion(videoId: string) {
    const version = (autoRemoveVersions.get(videoId) ?? 0) + 1;
    autoRemoveVersions.set(videoId, version);
    return version;
  }

  function cancelDeferredWatchlistRemoval(videoId: string): number | null {
    const removal = deferredWatchlistRemovals.get(videoId);
    if (!removal) return null;
    window.clearTimeout(removal.timer);
    deferredWatchlistRemovals.delete(videoId);
    return Math.max(0, removal.deadline - Date.now());
  }

  function invalidateDeferredWatchlistRemoval(videoId: string): number | null {
    const remainingDelay = cancelDeferredWatchlistRemoval(videoId);
    nextAutoRemoveVersion(videoId);
    return remainingDelay;
  }

  function scheduleDeferredWatchlistRemoval(
    videoId: string,
    delayMs = manualWatchFilterDelayMs,
  ) {
    cancelDeferredWatchlistRemoval(videoId);
    const version = nextAutoRemoveVersion(videoId);
    const timer = window.setTimeout(() => {
      deferredWatchlistRemovals.delete(videoId);
      if (hasAutoRemoveWatchlistItem(videoId))
        void trackRemoval(videoId, version);
    }, delayMs);
    deferredWatchlistRemovals.set(videoId, {
      timer,
      deadline: Date.now() + delayMs,
      version,
    });
  }

  async function removeFromAutoRemoveWatchlists(
    videoId: string,
    version: number,
  ): Promise<boolean> {
    error = null;
    watchlistLoadSequence += 1;
    const snapshots = displayedWatchlists
      .filter(
        (watchlist) =>
          watchlist.autoRemoveWatched &&
          watchlist.items.some((item) => item.video.videoId === videoId),
      )
      .map((watchlist) => ({
        watchlistId: watchlist.id,
        manualPosition: watchlist.items.find(
          (item) => item.video.videoId === videoId,
        )!.manualPosition,
      }));
    if (snapshots.length === 0) return true;

    try {
      const outcomes = await Promise.all(
        snapshots.map(async ({ watchlistId }) => {
          const key = membershipKey(watchlistId, videoId);
          const removed = await watchlistMembershipQueue.run(key, async () => {
            if (autoRemoveVersions.get(videoId) !== version) return false;
            await api.removeYoutubeVideoFromWatchlist(watchlistId, videoId);
            return true;
          });
          if (!removed) return { watchlistId, stale: true };
          return {
            watchlistId,
            stale: autoRemoveVersions.get(videoId) !== version,
          };
        }),
      );

      const removalWasInvalidated =
        autoRemoveVersions.get(videoId) !== version ||
        outcomes.some((outcome) => outcome.stale);
      if (removalWasInvalidated) {
        await Promise.all(
          snapshots.map(async ({ watchlistId, manualPosition }) => {
            const key = membershipKey(watchlistId, videoId);
            if (watchlistMembershipIntent.get(key) === false) return;
            await watchlistMembershipQueue.run(key, async () => {
              if (watchlistMembershipIntent.get(key) === false) return;
              await api.addYoutubeVideoToWatchlist(
                watchlistId,
                videoId,
                manualPosition,
              );
            });
          }),
        );
        await loadWatchlists(selectedWatchlistId, false);
        return true;
      }

      for (const { watchlistId } of snapshots) {
        localAdditions.delete(membershipKey(watchlistId, videoId));
        watchlistMembershipIntent.set(
          membershipKey(watchlistId, videoId),
          false,
        );
      }
      watchlists = watchlists.map((watchlist) => {
        if (
          !snapshots.some((snapshot) => snapshot.watchlistId === watchlist.id)
        ) {
          return watchlist;
        }
        return {
          ...watchlist,
          items: watchlist.items.filter(
            (item) => item.video.videoId !== videoId,
          ),
        };
      });
      return true;
    } catch (caught) {
      await loadWatchlists(selectedWatchlistId, false);
      error = normalizeError(caught);
      return false;
    }
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
    try {
      await api.reorderYoutubeWatchlist(watchlistId, videoIds);
      await loadWatchlists(watchlistId);
    } catch (caught) {
      const actionError = normalizeError(caught);
      await loadWatchlists(watchlistId, false);
      error = actionError;
    }
  }

  function applyProgress(progress: YoutubeProgressEvent): boolean {
    const manual = pendingManualWatchedVideoIds.delete(progress.videoId);
    const remove = watchlistVideoTransitionedToWatched(
      displayedWatchlists,
      progress.videoId,
      progress.isWatched,
    );
    if (!progress.isWatched)
      invalidateDeferredWatchlistRemoval(progress.videoId);
    updateWatchlistVideo(progress.videoId, (video) => ({
      ...video,
      positionSeconds: progress.positionSeconds,
      watchedPercentage: progress.watchedPercentage,
      isWatched: progress.isWatched,
    }));
    if (remove) {
      if (manual) scheduleDeferredWatchlistRemoval(progress.videoId);
      else {
        for (const watchlist of watchlists) {
          if (watchlist.autoRemoveWatched) {
            localAdditions.delete(
              membershipKey(watchlist.id, progress.videoId),
            );
          }
        }
        void loadWatchlists();
      }
    }
    return manual;
  }
  function reset() {
    watchlistLoadSequence += 1;
    takeDeferredWatchlistRemovals();
    pendingManualWatchedVideoIds.clear();
    localAdditions.clear();
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
    flushWatchlistRemovals,
    loadWatchlists,
    updateWatchlistVideo,
    hasAutoRemoveWatchlistItem,
    invalidateDeferredWatchlistRemoval,
    scheduleDeferredWatchlistRemoval,
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
