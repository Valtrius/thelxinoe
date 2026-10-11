import type { Page } from '@playwright/test';
import { installUiFixture } from './ui-fixture.ts';

export function sidebarArtwork(index: number) {
  const hue = (index * 47 + 185) % 360;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360"><defs><linearGradient id="g" x2="1" y2="1"><stop stop-color="hsl(${hue} 55% 32%)"/><stop offset="1" stop-color="hsl(${hue + 35} 40% 9%)"/></linearGradient></defs><path fill="url(#g)" d="M0 0h640v360H0z"/><circle cx="${180 + (index % 4) * 80}" cy="150" r="95" fill="hsl(${hue} 60% 70%)" opacity=".3"/><path d="M0 320 170 150 300 290 480 90 640 240v120H0" fill="hsl(${hue + 20} 45% 18%)"/><path d="M0 340 220 220 420 320 640 180v180H0" fill="hsl(${hue} 40% 12%)"/><text x="28" y="48" fill="white" opacity=".75" font-family="sans-serif" font-size="18" letter-spacing="4">${['FIELD NOTES', 'AFTER HOURS', 'OPEN STUDIO', 'ON THE ROAD'][index % 4]}</text></svg>`;
}

export async function installSidebarFixture(page: Page, section = 'YouTube') {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    section,
    preserveNavigation: true,
    settingsSection: 'server',
  });
  const now = Math.floor(Date.now() / 1000);
  const names = [
    'The long way home',
    'Inside the workshop',
    'A quiet morning',
    'Making something new',
    'Across the mountains',
    'The city after dark',
    'Small discoveries',
    'Building from scratch',
  ];
  const videos = Array.from({ length: 144 }, (_, index) => ({
    videoId: `preview${String(index).padStart(4, '0')}`,
    channelId: `channel-${index % 8}`,
    channelName: [
      'Field Notes',
      'Open Studio',
      'Technology Lab',
      'Slow Travel',
      'After Hours',
      'Everyday Science',
      'Sound Room',
      'The Workshop',
    ][index % 8],
    title: `${names[index % names.length]} · ${Math.floor(index / 8) + 1}`,
    thumbnailUrl: `/__sidebar-art/${index}.svg`,
    publishedAt: new Date(
      (now - Math.floor(index / 18) * 86400 - (index % 18) * 1200) * 1000,
    ).toISOString(),
    durationSeconds: 600 + (index % 9) * 180,
    broadcastState: 'none',
    isLive: false,
    isUpcoming: false,
    isLiveReplay: false,
    positionSeconds: index % 7 === 0 ? 180 : 0,
    watchedPercentage: index % 7 === 0 ? 20 : 0,
    isWatched: index % 13 === 0,
    availabilityStatus: 'available',
  }));
  const library = Array.from({ length: 30 }, (_, index) => ({
    id: `media-${index}`,
    kind: 'movie',
    title: names[index % 8],
    year: 2026 - (index % 12),
    available: true,
    artwork_url: `/__sidebar-art/${index}.svg`,
    metadata: {},
  }));
  const lists = [
    {
      id: 1,
      name: 'Watch later',
      isDefault: true,
      autoDownload: false,
      autoRemoveWatched: false,
      sortMode: 'manual',
      sortDirection: 'desc',
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      items: videos.slice(1, 21).map((video, index) => ({
        video,
        addedAt: video.publishedAt,
        manualPosition: index,
      })),
    },
  ];
  let appearance = {
    theme: 'dark',
    sidebar_collapsed: false,
    card_columns: 6,
    fade_watched: true,
    thumbnail_fit: 'contain',
    audio_volume: 1,
    player_height: null,
    youtube_card_shortcuts: [],
    provider_preferences: { 'youtube-watchlist-sidebar-open': 'true' },
  };
  await page.route('**/__sidebar-art/*.svg', (route) =>
    route.fulfill({
      contentType: 'image/svg+xml',
      body: sidebarArtwork(
        Number(new URL(route.request().url()).pathname.match(/\d+/)?.[0] ?? 0),
      ),
    }),
  );
  await page.route('https://image.tmdb.org/**', (route) =>
    route.fulfill({ contentType: 'image/svg+xml', body: sidebarArtwork(3) }),
  );
  await page.route('**/api/v1/**', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.slice('/api/v1'.length);
    const json = (value: unknown) => route.fulfill({ json: value });
    if (path === '/me/appearance') {
      if (request.method() === 'PATCH') {
        const patch = request.postDataJSON();
        appearance = {
          ...appearance,
          ...patch,
          provider_preferences: {
            ...appearance.provider_preferences,
            ...patch.provider_preferences,
          },
        };
      }
      return json(appearance);
    }
    if (['/online/youtube', '/online/twitch'].includes(path))
      return json({
        configured: true,
        linking_available: true,
        downloads_enabled: true,
        account: { status: 'connected', display_name: 'Preview viewer' },
        sync: { last_complete: now, next_run: now + 3600, error: null },
      });
    if (path === '/online/youtube/browse') {
      const { query, page: pageIndex, pageSize } = request.postDataJSON();
      const matching = videos.filter(
        (video) =>
          !query.search ||
          `${video.title} ${video.channelName}`
            .toLowerCase()
            .includes(query.search.toLowerCase()),
      );
      return json({
        items: matching.slice(pageIndex * pageSize, (pageIndex + 1) * pageSize),
        page: pageIndex,
        pageSize,
        hasMore: (pageIndex + 1) * pageSize < matching.length,
        counts: {
          all: videos.length,
          unwatched: 110,
          inProgress: 22,
          watched: 12,
          shorts: 0,
          live: 0,
          liveReplays: 0,
          upcoming: 0,
          subscribedChannelCount: 8,
        },
        channels: videos.slice(0, 8).map((video) => ({
          channelId: video.channelId,
          title: video.channelName,
        })),
      });
    }
    if (path === '/online/youtube/watchlists') return json(lists);
    if (path === '/online/youtube/home')
      return json({
        continue_watching: videos
          .slice(0, 8)
          .map((video) => ({ ...video, last_played: now })),
        next_up: videos.slice(8, 20),
      });
    if (path === '/me/home')
      return json({
        continue_watching: library.slice(0, 8).map((item) => ({
          ...item,
          position: 120,
          duration: 600,
          last_played: now,
        })),
        next_up: library.slice(8, 16).map((item) => ({
          ...item,
          kind: 'episode',
          show_title: 'Field Notes',
        })),
        watch_later: library.slice(16, 28),
        favorites: library.slice(4, 16),
      });
    if (path === '/catalog') {
      const domain = url.searchParams.get('domain');
      return json({
        items: library.map((item) => ({
          ...item,
          kind:
            domain === 'Music'
              ? 'album'
              : domain === 'Shows'
                ? 'show'
                : 'movie',
        })),
      });
    }
    if (path === '/catalog/roots')
      return json({ items: [], media_mount: '/preview' });
    if (path === '/online/twitch/feed')
      return json({
        items: library.slice(0, 24).map((item, index) => ({
          id: item.id,
          login: `studio${index}`,
          display_name: `Studio ${index + 1}`,
          title: item.title,
          category: index % 2 ? 'Science & Technology' : 'Just Chatting',
          viewers: 1200 + index * 317,
          thumbnail_url: item.artwork_url,
          started_at: new Date((now - 7200) * 1000).toISOString(),
          fetched_at: now,
        })),
      });
    if (path === '/online/kick')
      return json({
        connected: true,
        configured: true,
        items: library.slice(0, 24).map((item, index) => ({
          slug: `channel${index}`,
          display_name: `Channel ${index + 1}`,
          title: item.title,
          category: index % 2 ? 'IRL' : 'Gaming',
          live: index < 20,
          viewers: 800 + index * 211,
          thumbnail_url: item.artwork_url,
          started_at: new Date((now - 3600) * 1000).toISOString(),
          updated_at: now,
          error: null,
        })),
      });
    if (path === '/seerr/status')
      return json({ configured: true, ready: true });
    if (path.startsWith('/seerr/discover/') || path === '/seerr/search')
      return json({
        page: 1,
        totalPages: 1,
        totalResults: 20,
        results: library.slice(0, 20).map((item, index) => ({
          id: index + 1,
          mediaType: path.endsWith('tv') ? 'tv' : 'movie',
          title: item.title,
          name: item.title,
          posterPath: `/preview${index}.jpg`,
          backdropPath: '/preview.jpg',
          overview: 'An independent film following life, work, and travel.',
          releaseDate: '2026-10-01',
          voteAverage: 7.4,
        })),
      });
    if (path === '/playlists')
      return json({
        items: library.slice(0, 6).map((item, index) => ({
          id: item.id,
          name: [
            'Late nights',
            'On the road',
            'Focus',
            'Sunday morning',
            'Favorites',
            'New discoveries',
          ][index],
          description: '',
          owner_id: 'layout-fixture',
          owner: 'Layout viewer',
          favorite: false,
          count: 12,
          revision: 1,
        })),
      });
    if (path.endsWith('/history'))
      return json({ items: [], next_before: null });
    if (path.endsWith('/statistics')) {
      const sources = ['youtube', 'twitch', 'kick', 'movies', 'shows', 'music'];
      const seconds = Object.fromEntries(
        sources.map((source, index) => [`${source}Seconds`, (index + 1) * 900]),
      );
      return json({
        range: url.searchParams.get('range') || '30d',
        platform: url.searchParams.get('platform') || 'all',
        timezone: 'UTC',
        interval: 'day',
        trackingStartedAt: '2026-09-01T00:00:00Z',
        totalActiveSeconds: 18900,
        activeDays: 21,
        periodDays: 30,
        averageActiveSecondsPerDay: 900,
        youtubeVideosStarted: 42,
        youtubeVideosWatched: 31,
        twitchChannelsWatched: 8,
        kickChannelsWatched: 6,
        moviesStarted: 12,
        moviesWatched: 9,
        episodesStarted: 24,
        episodesWatched: 22,
        showsWatched: 4,
        tracksStarted: 180,
        tracksCompleted: 160,
        artistsListened: 12,
        albumsListened: 20,
        activity: Array.from({ length: 30 }, (_, index) => ({
          ...seconds,
          periodStart: `2026-09-${String(index + 1).padStart(2, '0')}`,
        })),
        rhythm: Array.from({ length: 168 }, (_, index) => ({
          ...seconds,
          weekday: Math.floor(index / 24),
          hour: index % 24,
        })),
        platformTotals: sources.map((platform, index) => ({
          platform,
          activeSeconds: (index + 1) * 900,
        })),
        topChannels: sources.map((platform) => ({
          platform,
          id: platform,
          name: platform,
          activeSeconds: 900,
          watchedDays: 12,
          contentCount: 20,
        })),
        youtubeContentMix: { uploads: 31, liveReplays: 8, shorts: 3 },
        users: [],
      });
    }
    return route.fallback();
  });
  return fixture;
}
