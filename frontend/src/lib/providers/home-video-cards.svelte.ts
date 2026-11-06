import { updateAppearance } from '../appearance';
import { copyText } from '../clipboard';
import { api, normalizeError } from './api';
import { dismissToastByKey, showToast } from './toasts';
import type { YoutubeDownload, YoutubeVideo } from './types';
import { youtubeUrl } from './utils';
import type { YoutubeVideoActions } from './youtube-video-actions';
import { createYoutubeWatchlists } from './youtube-watchlist-controller.svelte';

// What YouTube's video cards need on Home, which has no feed of its own to keep.
export function createHomeVideoCards(options: {
  play: (video: YoutubeVideo, beginning?: boolean) => void;
  downloadChanged: (videoId: string, download: YoutubeDownload) => void;
}) {
  const watchlists = createYoutubeWatchlists(0);
  let menu = $state<string | null>(null);
  let confirmingDelete = $state<string | null>(null);

  // Takes the video's identity because Undo runs after the card is gone.
  function failed(videoId: string, title: string, caught: unknown) {
    showToast({
      key: `youtube-video:${videoId}`,
      tone: 'error',
      title,
      message: normalizeError(caught).message,
    });
  }

  async function attempt(video: YoutubeVideo, action: () => Promise<unknown>) {
    const { videoId, title } = video;
    try {
      await action();
    } catch (caught) {
      failed(videoId, title, caught);
    }
  }

  async function changeDownload(
    video: YoutubeVideo,
    action: () => Promise<YoutubeDownload | void>,
  ) {
    const { videoId } = video;
    if (watchlists.downloadBusyIds.has(videoId)) return;
    watchlists.downloadBusyIds.add(videoId);
    await attempt(video, async () => {
      const download = await action();
      if (download) options.downloadChanged(videoId, download);
    });
    watchlists.downloadBusyIds.delete(videoId);
  }

  async function markWatched({
    videoId,
    title,
    positionSeconds,
    watchedPercentage,
  }: YoutubeVideo) {
    const key = `youtube-video:${videoId}`;
    try {
      await api.markYoutubeWatched(videoId);
    } catch (caught) {
      failed(videoId, title, caught);
      return;
    }
    // A watched video leaves Home, so Undo is where it is marked unwatched. It
    // keeps the progress that marking unwatched otherwise clears.
    showToast({
      key,
      tone: 'success',
      title,
      message: 'Marked watched.',
      actionLabel: 'Undo',
      onAction: () => {
        dismissToastByKey(key);
        api
          .markYoutubeUnwatched(videoId, { positionSeconds, watchedPercentage })
          .catch((caught) => failed(videoId, title, caught));
      },
    });
  }

  async function changeWatchlist(
    video: YoutubeVideo,
    change: () => Promise<unknown>,
  ) {
    await change();
    if (watchlists.error) failed(video.videoId, video.title, watchlists.error);
  }

  const actions: YoutubeVideoActions = {
    onPlay: (video) => options.play(video),
    onBeginning: (video) => options.play(video, true),
    onDownload: (video) =>
      void changeDownload(video, () => api.startYoutubeDownload(video.videoId)),
    onCancelDownload: (video) =>
      void changeDownload(video, () =>
        api.cancelYoutubeDownload(video.videoId),
      ),
    onPinDownload: (video) =>
      void changeDownload(video, () =>
        api.setYoutubeDownloadPinned(video.videoId, true),
      ),
    onUnpinDownload: (video) =>
      void changeDownload(video, () =>
        api.setYoutubeDownloadPinned(video.videoId, false),
      ),
    onDeleteDownload: (video) =>
      void changeDownload(video, () =>
        api.deleteYoutubeDownload(video.videoId),
      ),
    onMarkWatched: (video) => void markWatched(video),
    onMarkUnwatched: (video) =>
      void attempt(video, () => api.markYoutubeUnwatched(video.videoId)),
    onCopy: (video) =>
      void attempt(video, () => copyText(youtubeUrl(video.videoId))),
    onOpen: (video) =>
      void attempt(video, () => api.openExternal(youtubeUrl(video.videoId))),
    onAddToWatchlist: (video, watchlistId) =>
      void changeWatchlist(video, () =>
        watchlists.addToWatchlist(video, watchlistId),
      ),
    onRemoveFromWatchlist: (video, watchlistId) =>
      void changeWatchlist(video, () =>
        watchlists.removeFromWatchlist(watchlistId, video),
      ),
    onYoutubeCardShortcutsChanged: (youtube_card_shortcuts) =>
      updateAppearance({ youtube_card_shortcuts }),
  };

  return {
    actions,
    get watchlists() {
      return watchlists.watchlists;
    },
    loadWatchlists: () => void watchlists.loadWatchlists(null, false),
    downloadBusy: (videoId: string) => watchlists.downloadBusyIds.has(videoId),
    get menu() {
      return menu;
    },
    get confirmingDelete() {
      return confirmingDelete;
    },
    toggleMenu(videoId: string) {
      confirmingDelete = null;
      menu = menu === videoId ? null : videoId;
    },
    confirmDelete(videoId: string | null) {
      menu = null;
      confirmingDelete = videoId;
    },
    closeMenus() {
      menu = null;
      confirmingDelete = null;
    },
  };
}
