import { api, normalizeError } from './api';
import { FeedRequests } from './feed-requests';
import type {
  AppError,
  YoutubeVideo,
  YoutubeVideoCounts,
  YoutubeChannelOption,
  YoutubeFeedQuery,
} from './types';

export function createYoutubeFeed(options: {
  readonly key: string;
  readonly query: YoutubeFeedQuery;
  readonly connected: boolean;
  acceptChannels: (channels: YoutubeChannelOption[]) => boolean;
  prepareLayout: (reset: boolean) => () => Promise<void>;
  onError: (error: AppError | null) => void;
}) {
  const emptyCounts: YoutubeVideoCounts = {
    all: 0,
    unwatched: 0,
    inProgress: 0,
    watched: 0,
    shorts: 0,
    live: 0,
    liveReplays: 0,
    upcoming: 0,
    subscribedChannelCount: 0,
  };
  const maxRetainedPages = 5;
  let channels = $state<YoutubeChannelOption[]>([]);
  let videos = $state<YoutubeVideo[]>([]);
  let counts = $state<YoutubeVideoCounts>(emptyCounts);
  let firstPage = $state(0);
  let page = $state(0);
  let retainedPageSizes = $state<number[]>([]);
  let hasMore = $state(false);
  let loading = $state(true);
  let loadingMore = $state(false);
  const feedRequests = new FeedRequests();
  function loadVideos(reset: boolean, requestKey = options.key) {
    if (!reset && (loading || loadingMore)) return feedRequests.finishLoading();
    return feedRequests.load(async (ownsRequest) => {
      const current = () => ownsRequest() && requestKey === options.key;
      if (!options.connected) {
        videos = [];
        counts = emptyCounts;
        firstPage = 0;
        page = 0;
        retainedPageSizes = [];
        hasMore = false;
        loading = false;
        loadingMore = false;
        return;
      }
      if (reset) {
        loadingMore = false;
        loading = true;
        options.onError(null);
      } else {
        loadingMore = true;
      }
      try {
        const nextPage = reset ? 0 : page + 1;
        const result = await api.youtubeVideos(options.query, nextPage);
        if (!current()) return;
        channels = result.channels;
        if (!options.acceptChannels(result.channels)) return;

        const animate = options.prepareLayout(reset);
        if (reset) {
          videos = result.items;
          firstPage = result.page;
          retainedPageSizes = [result.items.length];
        } else {
          let nextVideos = [...videos, ...result.items];
          const nextPageSizes = [...retainedPageSizes, result.items.length];
          let nextFirstPage = firstPage;
          while (nextPageSizes.length > maxRetainedPages) {
            nextVideos = nextVideos.slice(nextPageSizes.shift() ?? 0);
            nextFirstPage += 1;
          }
          videos = nextVideos;
          firstPage = nextFirstPage;
          retainedPageSizes = nextPageSizes;
        }
        counts = result.counts;
        page = result.page;
        hasMore = result.hasMore;
        await animate();
      } catch (caught) {
        if (current()) options.onError(normalizeError(caught));
      } finally {
        if (current()) {
          loading = false;
          loadingMore = false;
        }
      }
    });
  }

  function removeRetainedItem(index: number) {
    let pageOffset = 0;
    retainedPageSizes = retainedPageSizes.map((pageSize) => {
      const containsItem = index >= pageOffset && index < pageOffset + pageSize;
      pageOffset += pageSize;
      return containsItem ? Math.max(0, pageSize - 1) : pageSize;
    });
  }

  async function refreshLoadedVideos(requestKey = options.key) {
    await feedRequests.finishLoading();
    if (!options.connected || requestKey !== options.key) return;
    if (videos.length === 0) {
      await loadVideos(true, requestKey);
      return;
    }
    const ownsRequest = feedRequests.begin();
    const current = () => ownsRequest() && requestKey === options.key;
    const firstLoadedPage = firstPage;
    const lastLoadedPage = page;

    try {
      const pages = await Promise.all(
        Array.from(
          { length: lastLoadedPage - firstLoadedPage + 1 },
          (_, index) =>
            api.youtubeVideos(options.query, firstLoadedPage + index),
        ),
      );
      if (!current()) return;
      const animate = options.prepareLayout(false);
      videos = pages.flatMap((result) => result.items);
      counts = pages[0]?.counts ?? counts;
      firstPage = pages[0]?.page ?? 0;
      page = pages.at(-1)?.page ?? firstPage;
      retainedPageSizes = pages.map((result) => result.items.length);
      hasMore = pages.at(-1)?.hasMore ?? false;
      await animate();
    } catch (caught) {
      if (current()) options.onError(normalizeError(caught));
    }
  }
  return {
    get videos() {
      return videos;
    },
    set videos(value: typeof videos) {
      videos = value;
    },
    get counts() {
      return counts;
    },
    set counts(value: typeof counts) {
      counts = value;
    },
    get firstPage() {
      return firstPage;
    },
    set firstPage(value: typeof firstPage) {
      firstPage = value;
    },
    get page() {
      return page;
    },
    set page(value: typeof page) {
      page = value;
    },
    get retainedPageSizes() {
      return retainedPageSizes;
    },
    set retainedPageSizes(value: typeof retainedPageSizes) {
      retainedPageSizes = value;
    },
    get hasMore() {
      return hasMore;
    },
    set hasMore(value: typeof hasMore) {
      hasMore = value;
    },
    get loading() {
      return loading;
    },
    set loading(value: typeof loading) {
      loading = value;
    },
    get loadingMore() {
      return loadingMore;
    },
    set loadingMore(value: typeof loadingMore) {
      loadingMore = value;
    },
    get channels() {
      return channels;
    },
    set channels(value: typeof channels) {
      channels = value;
    },
    loadVideos,
    refreshLoadedVideos,
    removeRetainedItem,
    dispose() {
      feedRequests.invalidate();
    },
  };
}
