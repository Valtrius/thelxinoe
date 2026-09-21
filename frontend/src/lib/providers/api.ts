// Provider views share their presentation and controllers across clients. All
// state and playback operations cross the authenticated server transport here.
import { invoke } from '@tauri-apps/api/core';
import { derived, get } from 'svelte/store';
import { appearance, updateAppearance } from '../appearance';
import { api as request, ApiError, desktop, serverUrl } from '../api';
import type { MediaChoice } from '../playback';
import type {
  AppError,
  KickChannel,
  KickSnapshot,
  TwitchLiveStream,
  YoutubeDownload,
  YoutubeDownloadEvent,
  YoutubeFeedQuery,
  YoutubeVideo,
  YoutubeVideoPage,
  YoutubeWatchlist,
  YoutubeWatchlistAddResult,
  YoutubeWatchlistUpdate,
} from './types';

let launch: ((choice: MediaChoice) => Promise<void>) | undefined;
let owner = '';
const videos = new Map<string, YoutubeVideo>();
const artworkExpires = new Map<string, number>();
const streams = new Map<string, { id: string; title: string }>();
export function connectPresentation(
  user: string,
  play: (choice: MediaChoice) => Promise<void>,
) {
  owner = user;
  launch = play;
  return () => {
    launch = undefined;
    videos.clear();
    artworkExpires.clear();
    streams.clear();
    owner = '';
  };
}
export function activeMediaId(choice: MediaChoice): string {
  const [platform, id] = choice.id.split(':');
  if (platform === 'twitch')
    return [...streams].find(([, stream]) => stream.id === id)?.[0] ?? id;
  return id;
}
export const preferences = {
  subscribe: derived(appearance, (value) => value.provider_preferences)
    .subscribe,
  getItem: (key: string) =>
    get(appearance).provider_preferences[key] ??
    localStorage.getItem(`thelxinoe:${serverUrl()}:${owner}:${key}`),
  setItem: (key: string, value: string) => {
    localStorage.setItem(`thelxinoe:${serverUrl()}:${owner}:${key}`, value);
    if (get(appearance).provider_preferences[key] !== value)
      updateAppearance({ provider_preferences: { [key]: value } });
  },
};
export function normalizeError(error: unknown): AppError {
  if (
    typeof error === 'object' &&
    error !== null &&
    'category' in error &&
    'message' in error
  )
    return error as AppError;
  const message = error instanceof Error ? error.message : String(error);
  return {
    category:
      error instanceof ApiError && error.status === 401
        ? 'authentication'
        : 'api',
    code: error instanceof ApiError ? error.code : 'request_failed',
    message,
    technical: '',
    action: 'Retry the action.',
  };
}
function remember(video: YoutubeVideo): YoutubeVideo {
  if (video.thumbnailUrl?.startsWith('/'))
    video.thumbnailUrl = serverUrl() + video.thumbnailUrl;
  const previous = videos.get(video.videoId);
  if (
    video.thumbnailUrl &&
    previous?.thumbnailUrl &&
    (artworkExpires.get(video.videoId) ?? 0) > Date.now()
  ) {
    video.thumbnailUrl = previous.thumbnailUrl;
  } else if (video.thumbnailUrl)
    artworkExpires.set(video.videoId, Date.now() + 240_000);
  videos.set(video.videoId, video);
  return video;
}
export const iso = (seconds?: number | null) =>
  seconds ? new Date(seconds * 1000).toISOString() : null;
export function downloadEvent(payload: {
  video_id: string;
  state: string;
  size?: number;
  downloaded_bytes: number;
  total_bytes?: number | null;
  eta_seconds?: number | null;
  media_kind?: 'video' | 'audio' | 'media' | null;
}): YoutubeDownloadEvent {
  const previous = videos.get(payload.video_id)?.download;
  return {
    download: {
      ...previous,
      videoId: payload.video_id,
      status: (['queued', 'downloading', 'ready'].includes(payload.state)
        ? payload.state
        : 'failed') as YoutubeDownload['status'],
      fileSizeBytes: payload.size ?? 0,
      pinned: previous?.pinned ?? false,
      quality: previous?.quality ?? '1080p',
      requestedAt: previous?.requestedAt ?? '',
    },
    downloadedBytes: payload.downloaded_bytes,
    totalBytes: payload.total_bytes,
    etaSeconds: payload.eta_seconds,
    mediaKind: payload.media_kind,
  };
}
export type OnlineAccount = {
  configured: boolean;
  linking_available?: boolean;
  linking_url?: string | null;
  downloads_enabled?: boolean;
  quota?: { blocked: boolean };
  account: {
    status: string;
    display_name: string;
    avatar_url?: string | null;
    external_id?: string;
    updated_at?: number;
  };
  pending?: {
    user_code: string;
    verification_uri: string;
    expires_at: number;
    error: string | null;
  } | null;
  sync?: {
    last_complete: number | null;
    next_run: number;
    error: string | null;
    in_progress?: boolean;
    phase?: string;
  } | null;
};
export type KickFeed = {
  connected: boolean;
  configured: boolean;
  items: {
    slug: string;
    display_name?: string;
    title: string;
    category: string;
    live: boolean | null;
    viewers: number;
    thumbnail_url?: string;
    profile_image_url?: string;
    started_at?: string;
    updated_at: number;
    error: string | null;
    language?: string;
    mature?: boolean;
    tags?: string[];
  }[];
};
export function kickChannels(feed: KickFeed): KickChannel[] {
  return feed.items.map((s) => ({
    slug: s.slug,
    displayName: s.display_name || s.slug,
    profilePictureUrl: s.profile_image_url,
    liveStream: s.live
      ? {
          streamId: `${s.slug}:${s.started_at ?? ''}`,
          title: s.title,
          categoryName: s.category,
          thumbnailUrl: s.thumbnail_url,
          viewerCount: s.viewers,
          startedAt: s.started_at ?? '',
          fetchedAt: iso(s.updated_at) ?? '',
          languageCode: s.language,
          hasMatureContent: s.mature ?? false,
          tags: s.tags ?? [],
        }
      : null,
  }));
}
async function getDownload(videoId: string): Promise<YoutubeDownload> {
  const result = await request<{
    download: { state: string; size?: number; error?: string } | null;
  }>(`/online/youtube/videos/${videoId}/download`);
  const d = result.download;
  if (!d) throw new Error('This download is no longer available.');
  return {
    videoId,
    status: (['queued', 'downloading', 'processing', 'ready'].includes(d.state)
      ? d.state
      : 'failed') as YoutubeDownload['status'],
    fileSizeBytes: d.size ?? 0,
    pinned: videos.get(videoId)?.download?.pinned ?? false,
    quality: '1080p',
    requestedAt: new Date().toISOString(),
    errorMessage: d.error,
  };
}
async function mark(videoId: string, watched: boolean) {
  await request(`/online/youtube/videos/${videoId}`, 'PUT', { watched });
  const video = videos.get(videoId);
  window.dispatchEvent(
    new CustomEvent('thelxinoe-youtube-progress', {
      detail: {
        videoId,
        positionSeconds: video?.positionSeconds ?? 0,
        durationSeconds: video?.durationSeconds,
        watchedPercentage: video?.watchedPercentage ?? 0,
        isWatched: watched,
      },
    }),
  );
}
export const api = {
  async youtubeVideos(
    query: YoutubeFeedQuery,
    page: number,
    pageSize = 60,
  ): Promise<YoutubeVideoPage> {
    const result = await request<YoutubeVideoPage>(
      '/online/youtube/browse',
      'POST',
      { query, page, pageSize },
    );
    result.items.forEach(remember);
    return result;
  },
  async youtubeWatchlists(): Promise<YoutubeWatchlist[]> {
    const lists = await request<YoutubeWatchlist[]>(
      '/online/youtube/watchlists',
    );
    for (const list of lists)
      for (const item of list.items) remember(item.video);
    return lists;
  },
  createYoutubeWatchlist: (name: string) =>
    request<number>('/online/youtube/watchlists', 'POST', { name }),
  updateYoutubeWatchlist: async (
    id: number,
    update: YoutubeWatchlistUpdate,
  ) => {
    await request(`/online/youtube/watchlists/${id}`, 'PUT', update);
  },
  deleteYoutubeWatchlist: async (id: number) => {
    await request(`/online/youtube/watchlists/${id}`, 'DELETE');
  },
  async addYoutubeVideoToWatchlist(
    id: number,
    videoId: string,
    manualPosition: number,
  ) {
    const result = await request<YoutubeWatchlistAddResult>(
      `/online/youtube/watchlists/${id}/items`,
      'POST',
      { videoId, manualPosition },
    );
    remember(result.video);
    return result;
  },
  removeYoutubeVideoFromWatchlist: async (id: number, videoId: string) => {
    await request(
      `/online/youtube/watchlists/${id}/items/${videoId}`,
      'DELETE',
    );
  },
  reorderYoutubeWatchlist: async (id: number, videoIds: string[]) => {
    await request(`/online/youtube/watchlists/${id}/order`, 'PUT', {
      videoIds,
    });
  },
  async connectYoutube() {
    if (desktop) {
      await invoke('open_youtube_linking');
      return 'youtube';
    }
    const account = await request<OnlineAccount>('/online/youtube');
    if (
      account.linking_url &&
      new URL(account.linking_url).origin !== location.origin
    ) {
      location.assign(account.linking_url);
      return 'youtube';
    }
    const { url } = await request<{ url: string }>(
      '/online/youtube/connect',
      'POST',
    );
    const target = new URL(url);
    if (
      target.protocol !== 'https:' ||
      target.hostname !== 'accounts.google.com'
    )
      throw new Error('Invalid Google authorization address');
    location.assign(target.href);
    return 'youtube';
  },
  cancelYoutubeAuth: async (_id: string) => {
    void _id;
    await request('/online/youtube/authorization', 'DELETE');
    window.dispatchEvent(new Event('thelxinoe-provider-auth-cancelled'));
  },
  syncYoutube: async (mode: 'normal' | 'full') => {
    await request(`/online/youtube/sync?mode=${mode}`, 'POST');
    return 'youtube';
  },
  async startTwitchAuth() {
    await request('/online/twitch/connect', 'POST');
    return 'twitch';
  },
  cancelTwitchAuth: async (_id: string) => {
    void _id;
    await request('/online/twitch/authorization', 'DELETE');
  },
  syncTwitch: async () => {
    await request('/online/twitch/sync', 'POST');
    return 'twitch';
  },
  async twitchStreams(): Promise<TwitchLiveStream[]> {
    const result = await request<{
      items: {
        id: string;
        login: string;
        display_name: string;
        title: string;
        category: string;
        viewers: number;
        thumbnail_url?: string;
        profile_image_url?: string;
        started_at: string;
        fetched_at?: number;
      }[];
    }>('/online/twitch/feed');
    return result.items.map((s) => {
      streams.set(s.login, s);
      return {
        streamId: `${s.id}:${s.started_at}`,
        broadcasterId: s.id,
        login: s.login,
        displayName: s.display_name,
        title: s.title,
        gameName: s.category,
        viewerCount: s.viewers,
        thumbnailUrl: s.thumbnail_url,
        profileImageUrl: s.profile_image_url,
        startedAt: s.started_at,
        fetchedAt: iso(s.fetched_at) ?? '',
      };
    });
  },
  kickSnapshot: async (): Promise<KickSnapshot> => ({
    channels: kickChannels(await request<KickFeed>('/online/kick')),
  }),
  async addKickTrackedChannel(channel: string): Promise<KickChannel> {
    const added = await request<{ slug: string }>(
      '/online/kick/channels',
      'POST',
      { channel },
    );
    return { slug: added.slug, displayName: added.slug };
  },
  removeKickTrackedChannel: async (slug: string) => {
    await request(
      `/online/kick/channels/${encodeURIComponent(slug)}`,
      'DELETE',
    );
  },
  syncKick: async () => {
    await request('/online/kick/sync', 'POST');
    return 'kick';
  },
  async launchYoutube(
    videoId: string,
    startMode: 'resume' | 'beginning',
    _signedIn = false,
  ) {
    if (_signedIn)
      throw new Error(
        'Signed-in browser cookies are not supported by this server.',
      );
    if (!launch) throw new Error('Sign in before playing a video.');
    if (!videos.has(videoId))
      await request('/online/youtube/resolve', 'POST', { video_id: videoId });
    await launch({
      id: `youtube:${videoId}`,
      title: videos.get(videoId)?.title ?? videoId,
      position: startMode === 'beginning' ? 0 : undefined,
    });
  },
  async launchTwitch(login: string) {
    const stream = streams.get(login);
    if (!stream || !launch)
      throw new Error('Refresh this channel before playing.');
    await launch({ id: `twitch:${stream.id}`, title: stream.title });
  },
  async launchKick(slug: string) {
    if (!launch) throw new Error('Sign in before playing a channel.');
    await launch({ id: `kick:${slug}`, title: slug });
  },
  async startYoutubeDownload(videoId: string, _signedIn = false) {
    if (_signedIn)
      throw new Error(
        'Signed-in browser cookies are not supported by this server.',
      );
    // A user's retained interest protects the server's shared downloaded file.
    await request(`/online/youtube/videos/${videoId}`, 'PUT', { pinned: true });
    await request(`/online/youtube/videos/${videoId}/download`, 'POST');
    const download = await getDownload(videoId);
    download.pinned = true;
    return download;
  },
  cancelYoutubeDownload: async (id: string) => {
    await request(`/online/youtube/videos/${id}/download`, 'DELETE');
    window.dispatchEvent(
      new CustomEvent('thelxinoe-youtube-download-removed', { detail: id }),
    );
  },
  deleteYoutubeDownload: async (id: string) => {
    await request(`/online/youtube/videos/${id}/download`, 'DELETE');
    window.dispatchEvent(
      new CustomEvent('thelxinoe-youtube-download-removed', { detail: id }),
    );
  },
  async setYoutubeDownloadPinned(videoId: string, pinned: boolean) {
    await request(`/online/youtube/videos/${videoId}`, 'PUT', { pinned });
    const download = await getDownload(videoId);
    download.pinned = pinned;
    return download;
  },
  markYoutubeWatched: (id: string) => mark(id, true),
  markYoutubeUnwatched: (id: string) => mark(id, false),
  async openExternal(url: string) {
    if (desktop) await invoke('open_provider_url', { value: url });
    else window.open(url, '_blank', 'noopener,noreferrer');
  },
};
