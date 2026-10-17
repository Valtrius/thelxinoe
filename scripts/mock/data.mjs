import { titles } from '../density-fixtures.mjs';

export const now = () => Math.floor(Date.now() / 1000);
export const iso = (offset = 0) =>
  new Date((now() + offset) * 1000).toISOString();
export const art = (kind, index) => `/mock-art/${kind}/${index}`;
export const admin = {
  id: 'layout-fixture',
  username: 'Preview admin',
  role: 'admin',
  timezone: 'Europe/Paris',
};
export const member = {
  id: 'user-alex',
  username: 'Alex',
  role: 'user',
  timezone: 'Europe/Paris',
};
// The preview signs in as the administrator or as a regular member.
export const viewer = (s) =>
  s.users.find(
    (user) => user.id === (s.viewer === 'user' ? member.id : admin.id),
  ) ?? s.users[0];

export function librarySeed() {
  const items = [];
  function add(kind, index, extra = {}) {
    const item = {
      id: `${kind}-${index}`,
      kind,
      title: `${titles[index % titles.length]}${index >= 12 ? ' II' : ''}`,
      year: 2026 - (index % 7),
      available: true,
      artwork_url: art(kind, index),
      overview:
        'A remote observatory receives an unexpected signal. Its crew follows the clues across a landscape transformed by the changing seasons.',
      metadata: {
        genres: ['Drama', 'Adventure'],
        overview:
          'An expedition follows a mysterious signal beyond the familiar horizon.',
        rating: 7.8,
      },
      duration: kind === 'track' ? 240 : kind === 'episode' ? 2700 : 6600,
      position: 0,
      fileId: `file-${kind}-${index}`,
      ...extra,
    };
    items.push(item);
    return item;
  }
  for (let i = 0; i < 24; i++) {
    add('movie', i, { position: i < 3 ? 1200 + i * 900 : 0 });
    const show = add('show', i);
    const artist = add('artist', i);
    for (let season = 1; season <= 2; season++) {
      const parent = add('season', i * 2 + season - 1, {
        title: `Season ${season}`,
        parent_id: show.id,
        metadata: { season_number: season },
      });
      for (let episode = 1; episode <= 4; episode++) {
        add('episode', i * 8 + (season - 1) * 4 + episode - 1, {
          title: `${show.title} · ${['First Contact', 'The Crossing', 'The Signal', 'Homeward'][episode - 1]}`,
          parent_id: parent.id,
          show_title: show.title,
          artwork_url: art('video', episode + i),
          metadata: { season_number: season, episode_number: episode },
          position: i === 0 && season === 1 && episode === 1 ? 720 : 0,
        });
      }
    }
    for (let album = 0; album < 2; album++) {
      const parent = add('album', i * 2 + album, {
        parent_id: artist.id,
        title: `${artist.title} · ${album ? 'Night Sessions' : 'Studio Sessions'}`,
        metadata: { artist: artist.title },
      });
      for (let track = 0; track < 5; track++)
        add('track', i * 10 + album * 5 + track, {
          parent_id: parent.id,
          title: [
            'Daybreak',
            'Open Roads',
            'The Long Way',
            'Still Waters',
            'Afterglow',
          ][track],
          artwork_url: parent.artwork_url,
          metadata: {
            artist: artist.title,
            album: parent.title,
            track_number: track + 1,
          },
        });
    }
  }
  return items;
}

export function discoverSeed() {
  const items = [];
  for (const kind of ['movie', 'tv'])
    for (let i = 0; i < 24; i++) {
      const id = kind === 'movie' ? 11 + i : 22 + i;
      const title =
        kind === 'tv' && i === 0
          ? 'Northern Lights'
          : `${titles[i % titles.length]}${i >= 12 ? ' II' : ''}`;
      items.push({
        id,
        mediaType: kind,
        ...(kind === 'movie' ? { title } : { name: title }),
        overview:
          'A distant signal draws a team of explorers into an unfamiliar world. As the journey unfolds, friendships are tested and an unexpected discovery changes the way they see home.',
        tagline: 'Every journey begins with a signal.',
        posterPath: art(kind === 'movie' ? 'movie' : 'show', i),
        backdropPath: art('video', i),
        releaseDate: `202${6 - (i % 5)}-09-12`,
        firstAirDate: `202${6 - (i % 5)}-03-08`,
        voteAverage: 7.2 + (i % 7) / 10,
        runtime: 118,
        numberOfSeasons: kind === 'tv' ? 3 : undefined,
        status: kind === 'tv' ? 'Returning Series' : 'Released',
        genres: [
          { id: 18, name: 'Drama' },
          { id: 878, name: 'Science Fiction' },
        ],
        productionCompanies: [{ id: 1, name: 'Horizon Pictures' }],
        networks: [{ id: 1, name: 'Northstar' }],
        credits: {
          cast: [
            'Alex Morgan',
            'Sam Rivera',
            'Jordan Lee',
            'Robin Chen',
            'Taylor Brooks',
            'Casey Martin',
          ].map((name, index) => ({
            id: index + 1,
            name,
            character: [
              'Dr. Rowan',
              'Captain Hale',
              'Ellis',
              'Avery',
              'Quinn',
              'Jamie',
            ][index],
            profilePath: art('person', index),
          })),
        },
        relatedVideos: [],
        ...(kind === 'tv'
          ? {
              seasons: [0, 1, 2, 3].map((seasonNumber) => ({
                id: id * 10 + seasonNumber,
                seasonNumber,
                name: seasonNumber ? `Season ${seasonNumber}` : 'Specials',
                episodeCount: seasonNumber ? 8 : 2,
                airDate: '2026-03-08',
                posterPath: art('show', i + seasonNumber),
              })),
            }
          : {}),
        mediaInfo: {
          status: i === 0 ? 1 : [1, 5, 3, 2, 4, 6][i % 6],
          seasons:
            kind === 'tv' && i === 0 ? [{ seasonNumber: 1, status: 5 }] : [],
          requests: [],
        },
      });
    }
  return items;
}

export function videoSeed() {
  return Array.from({ length: 30 }, (_, index) => ({
    videoId: `preview${String(index).padStart(4, '0')}`,
    channelId: `channel-${index % 3}`,
    channelName: ['Field Notes', 'Night Studio', 'Open Roads'][index % 3],
    title: `${titles[index % titles.length]} · Episode ${index + 1}`,
    thumbnailUrl: art('video', index),
    publishedAt: iso(-index * 7200),
    durationSeconds: index % 10 === 8 ? 120 : 600 + index * 60,
    broadcastState:
      index === 28
        ? 'upcoming'
        : index === 29
          ? 'live'
          : index % 9 === 7
            ? 'replay'
            : 'none',
    isLive: index === 29,
    isUpcoming: index === 28,
    isLiveReplay: index % 9 === 7,
    isShort: index % 10 === 8,
    positionSeconds: index < 3 ? 180 : 0,
    watchedPercentage: index < 3 ? 30 : index > 20 && index < 26 ? 100 : 0,
    isWatched: index > 20 && index < 26,
    availabilityStatus: 'available',
    download:
      index < 2
        ? {
            videoId: `preview${String(index).padStart(4, '0')}`,
            status: 'ready',
            fileSizeBytes: 540 * 1024 ** 2,
            pinned: index === 0,
            quality: '1080p',
            requestedAt: iso(-3600),
          }
        : null,
  }));
}

export const definitions = [
  ['seerr', 5055, 'support'],
  ['recyclarr', null, 'job'],
  ['radarr', 7878, 'manager'],
  ['sonarr', 8989, 'manager'],
  ['lidarr', 8686, 'manager'],
  ['bazarr', 6767, 'support'],
  ['prowlarr', 9696, 'support'],
  ['nzbget', 6789, 'support'],
];
export const label = (kind) =>
  kind === 'nzbget' ? 'NZBGet' : kind[0].toUpperCase() + kind.slice(1);

export function statistics(query, users) {
  const platforms = ['youtube', 'twitch', 'kick', 'movies', 'shows', 'music'];
  const platform = query.get('platform') ?? 'all';
  const range = query.get('range') ?? '30d';
  const count = { '7d': 7, '30d': 30, '90d': 90, all: 120 }[range] ?? 30;
  const seconds = (seed) =>
    Object.fromEntries(
      platforms.map((p, index) => [
        `${p}Seconds`,
        platform === 'all' || platform === p
          ? (((seed * (index + 3)) % 12) + 1) * 240
          : 0,
      ]),
    );
  const activity = Array.from({ length: count }, (_, i) => ({
    periodStart: iso(-(count - i - 1) * 86400).slice(0, 10),
    ...seconds(i + 1),
  }));
  const platformTotals = platforms.map((p) => ({
    platform: p,
    activeSeconds: activity.reduce(
      (sum, point) => sum + point[`${p}Seconds`],
      0,
    ),
  }));
  const totalActiveSeconds = platformTotals.reduce(
    (sum, p) => sum + p.activeSeconds,
    0,
  );
  return {
    range,
    platform,
    timezone: 'Europe/Paris',
    interval: 'day',
    trackingStartedAt: iso(-120 * 86400),
    totalActiveSeconds,
    activeDays: count,
    periodDays: count,
    averageActiveSecondsPerDay: totalActiveSeconds / count,
    youtubeVideosStarted: 46,
    youtubeVideosWatched: 38,
    twitchChannelsWatched: 8,
    kickChannelsWatched: 5,
    moviesStarted: 9,
    moviesWatched: 7,
    episodesStarted: 28,
    episodesWatched: 25,
    showsWatched: 4,
    tracksStarted: 106,
    tracksCompleted: 98,
    artistsListened: 12,
    albumsListened: 18,
    activity,
    rhythm: Array.from({ length: 168 }, (_, i) => ({
      weekday: Math.floor(i / 24),
      hour: i % 24,
      ...seconds(i + 1),
    })),
    platformTotals,
    topChannels: platforms.map((p, i) => ({
      platform: p,
      id: `top-${i}`,
      name: [
        'Field Notes',
        'Channel 1',
        'Creator 1',
        'Beyond the Horizon',
        'Northern Lights',
        'Night Studio',
      ][i],
      activeSeconds: platformTotals[i].activeSeconds,
      watchedDays: 8,
      contentCount: 12,
    })),
    youtubeContentMix: { uploads: 26, liveReplays: 8, shorts: 4 },
    users: users.map((u, i) => ({
      id: u.id,
      username: u.username,
      activeSeconds: Math.floor(totalActiveSeconds / (i + 1)),
    })),
  };
}
