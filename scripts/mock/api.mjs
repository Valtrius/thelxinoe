import {
  admin,
  art,
  discoverSeed,
  iso,
  librarySeed,
  now,
  videoSeed,
} from './data.mjs';
import { mediaApi, onlineApi } from './media.mjs';
import { servicesApi, settingsApi, settingsSeed } from './settings.mjs';
import { playbackApi } from './playback.mjs';
import { scenarioSeed, serviceAttention } from './scenarios.mjs';
import { waitForServerAction } from './timing.mjs';

export function createMockState() {
  const library = librarySeed();
  const discover = discoverSeed();
  const videos = videoSeed();
  const requests = discover
    .filter((m) => [2, 3].includes(m.mediaInfo.status))
    .slice(0, 8)
    .map((media, i) => ({
      id: i + 1,
      status: media.mediaInfo.status === 2 ? 1 : 2,
      createdAt: iso(-i * 86400),
      type: media.mediaType,
      requestedBy: {
        id: i % 2 ? 2 : 1,
        username: i % 2 ? 'Alex' : admin.username,
      },
      seasons: media.mediaType === 'tv' ? [{ seasonNumber: 1, status: 2 }] : [],
      media: {
        tmdbId: media.id,
        mediaType: media.mediaType,
        status: media.mediaInfo.status,
      },
    }));
  for (const media of discover)
    media.mediaInfo.requests = requests.filter(
      (r) => r.type === media.mediaType && r.media.tmdbId === media.id,
    );
  const streams = (kind) =>
    Array.from({ length: 16 }, (_, i) => ({
      ...(kind === 'twitch'
        ? { id: `twitch-${i}`, login: `channel${i}` }
        : { slug: `creator${i}`, live: i < 13 }),
      display_name: kind === 'twitch' ? `Channel ${i + 1}` : `Creator ${i + 1}`,
      title: [
        'Exploring the northern wilderness',
        'Live from the studio',
        'Building an observatory',
        'Evening games',
      ][i % 4],
      category: ['Science & Technology', 'Music', 'Just Chatting', 'Gaming'][
        i % 4
      ],
      viewers: 750 + i * 137,
      thumbnail_url: art('video', i),
      profile_image_url: art('person', i),
      started_at: iso(-3600),
      fetched_at: now(),
      updated_at: now(),
      error: null,
      language: 'en',
      mature: false,
      tags: ['English', 'Community'],
    }));
  return {
    ...settingsSeed(),
    scenarios: scenarioSeed(),
    signedIn: true,
    sequence: 100,
    library,
    discover,
    requests,
    videos,
    videoTemplate: structuredClone(videos[5]),
    users: [
      { ...admin },
      {
        id: 'user-alex',
        username: 'Alex',
        role: 'user',
        timezone: 'Europe/Paris',
      },
      { id: 'user-robin', username: 'Robin', role: 'user', timezone: 'UTC' },
    ],
    mediaStates: Object.fromEntries(
      ['movie-4', 'movie-5', 'show-1', 'artist-0'].map((id) => [
        id,
        { favorite: true, watch_later: id.startsWith('movie'), watched: false },
      ]),
    ),
    playlists: [
      {
        id: 'playlist-1',
        name: 'Evening listening',
        description: 'Studio sessions and quiet tracks',
        owner_id: admin.id,
        owner: admin.username,
        favorite: true,
        revision: 1,
        items: library.filter((item) => item.kind === 'track').slice(0, 12),
      },
      {
        id: 'playlist-2',
        name: 'On the road',
        description: '',
        owner_id: admin.id,
        owner: admin.username,
        favorite: false,
        revision: 1,
        items: library.filter((item) => item.kind === 'track').slice(12, 22),
      },
    ],
    watchlists: [
      {
        id: 1,
        name: 'Watch Later',
        isDefault: true,
        autoDownload: false,
        autoRemoveWatched: false,
        sortMode: 'manual',
        sortDirection: 'asc',
        createdAt: iso(-7 * 86400),
        updatedAt: iso(),
        items: videos.slice(4, 10).map((video, manualPosition) => ({
          videoId: video.videoId,
          video,
          manualPosition,
          addedAt: iso(-manualPosition * 3600),
        })),
      },
      {
        id: 2,
        name: 'Weekend videos',
        isDefault: false,
        autoDownload: false,
        autoRemoveWatched: false,
        sortMode: 'manual',
        sortDirection: 'asc',
        createdAt: iso(-3 * 86400),
        updatedAt: iso(),
        items: videos.slice(10, 14).map((video, manualPosition) => ({
          videoId: video.videoId,
          video,
          manualPosition,
          addedAt: iso(),
        })),
      },
    ],
    connected: { youtube: true, twitch: true, kick: true },
    twitch: streams('twitch'),
    kick: streams('kick'),
    approvals: {},
    queues: {},
    playback: {},
    segmentsByMedia: {},
    operations: {},
    preferences: { timezone_override: null, time_format_override: null },
    appearance: {
      provider_preferences: {},
      audio_volume: 1,
      player_height: null,
      youtube_card_shortcuts: [],
      theme: 'light',
      sidebar_collapsed: false,
      poster_columns: 6,
      video_columns: 6,
      animation_duration_ms: 200,
      fade_watched: true,
      thumbnail_fit: 'contain',
    },
  };
}

export async function installMockApi(page, currentState, events) {
  await page.route('**/api/v1/**', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.slice('/api/v1'.length);
    const s = currentState();
    const c = {
      path,
      method: request.method(),
      body: request.postDataJSON() ?? {},
      query: url.searchParams,
      origin: url.origin,
      emit: events.emit,
      async json(value) {
        const write =
          !['GET', 'HEAD'].includes(request.method()) &&
          ![
            '/auth/event-ticket',
            '/online/youtube/browse',
            '/admin/recyclarr/configuration/validate',
            '/auth/login',
          ].includes(path);
        if (write) {
          const change =
            path === '/me/preferences'
              ? 'preferences.changed'
              : path === '/admin/settings'
                ? 'server.settings.changed'
                : path === '/me/avatar'
                  ? 'account.profile.changed'
                  : path.startsWith('/playlists')
                    ? 'playlists.changed'
                    : /^\/catalog\/[^/]+\/state$/.test(path)
                      ? 'media-state.changed'
                      : path.startsWith('/catalog') ||
                          path.startsWith('/admin/media')
                        ? 'catalog.changed'
                        : path.startsWith('/admin/jobs')
                          ? 'jobs.changed'
                          : path.startsWith('/admin/product-update')
                            ? 'product.changed'
                            : path.startsWith('/admin/online')
                              ? 'online.configuration.changed'
                              : path.startsWith('/online/youtube/')
                                ? 'youtube.changed'
                                : path.startsWith('/online/')
                                  ? 'online.account.changed'
                                  : null;
          if (path === '/me/preferences') s.users[0].timezone = value.timezone;
          if (change)
            events.emit(change, change === 'preferences.changed' ? value : {});
          if (
            !['/me/appearance', '/playback'].includes(path) &&
            !path.startsWith('/playback/') &&
            !path.startsWith('/me/queue/')
          )
            s.audit.unshift({
              id: `audit-${++s.sequence}`,
              action: path.startsWith('/admin/')
                ? `${path.split('/')[2]}.update`
                : `${path.split('/')[1]}.update`,
              target: path,
              actor: admin.username,
              actor_id: admin.id,
              created_at: now(),
            });
        }
        await route.fulfill({ json: value });
        return true;
      },
      async fail(status, message) {
        await route.fulfill({
          status,
          json: { error: { message } },
          headers: { 'x-thelxinoe-mock-error': 'handled' },
        });
        return true;
      },
    };
    await waitForServerAction(s, c);
    if (path === '/health') return c.json({ api_version: 1 });
    if (path === '/setup') return c.json({ setup_required: false });
    if (path === '/timezones')
      return c.json({
        timezones: [
          'UTC',
          'Europe/Paris',
          'Europe/London',
          'America/New_York',
          'America/Los_Angeles',
          'Asia/Tokyo',
          'Australia/Sydney',
        ],
      });
    if (path === '/auth/event-ticket')
      return c.json({
        ticket: 'preview',
        epoch: 'preview',
        cursor: events.cursor(),
        version: 1,
      });
    if (path === '/auth/me')
      return s.signedIn
        ? c.json({ user: s.users[0] })
        : c.fail(401, 'Sign in required');
    if (path === '/auth/login') {
      s.signedIn = true;
      return c.json({ user: s.users[0] });
    }
    if (path === '/auth/logout') {
      s.signedIn = false;
      return c.json({ signed_out: true });
    }
    if (path === '/auth/methods')
      return c.json({
        canonical_url: url.origin,
        passkeys: true,
        oidc: s.oidc.provider
          ? { label: s.oidc.provider.label, available: true }
          : null,
        server_id: 'disposable-preview',
      });
    if (path === '/auth/passkey/finish') {
      s.signedIn = true;
      return c.json({ user: s.users[0], verified: true });
    }
    if (path === '/auth/oidc/start')
      return c.json({ url: `${url.origin}/__mock/auth` });
    if (path === '/me/attention') return c.json(serviceAttention(s));
    for (const handler of [
      settingsApi,
      servicesApi,
      playbackApi,
      mediaApi,
      onlineApi,
    ])
      if (await handler(s, c)) return;
    return route.fallback();
  });
}
