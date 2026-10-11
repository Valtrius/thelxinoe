export const titles = [
  'Beyond the Horizon',
  'Nightfall',
  'The Last Signal',
  'Northern Lights',
  'A Quiet Journey',
  'After the Rain',
  'Open Water',
  'Parallel Worlds',
  'The Observatory',
  'Golden Hour',
  'Into the Wild',
  'First Light',
];
const colors = [
  '#285773',
  '#684252',
  '#466854',
  '#65517b',
  '#9a6941',
  '#386a74',
];

export function artwork(kind, index) {
  const square = ['artist', 'album', 'track', 'person'].includes(kind);
  const height = square ? 480 : kind === 'video' ? 270 : 720;
  const title = titles[index % titles.length];
  return `<svg xmlns="http://www.w3.org/2000/svg" width="480" height="${height}" viewBox="0 0 480 ${height}">
    <defs><linearGradient id="sky" x2="0.7" y2="1"><stop stop-color="${colors[index % colors.length]}"/><stop offset="1" stop-color="#101723"/></linearGradient></defs>
    <rect width="480" height="${height}" fill="url(#sky)"/>
    <circle cx="${130 + (index % 4) * 65}" cy="${height * 0.3}" r="${45 + (index % 3) * 12}" fill="#e5cda4" opacity=".65"/>
    <path d="M0 ${height * 0.75}L140 ${height * 0.45}L260 ${height * 0.72}L390 ${height * 0.4}L480 ${height * 0.7}V${height}H0Z" fill="#101723" opacity=".65"/>
    <path d="M0 ${height * 0.85}L180 ${height * 0.65}L310 ${height * 0.8}L480 ${height * 0.6}V${height}H0Z" fill="#0a1019" opacity=".6"/>
    <text x="28" y="${height - 48}" fill="#f5eee4" font-family="sans-serif" font-size="${kind === 'video' ? 24 : 30}" font-weight="700">${title}</text>
    <text x="30" y="${height - 24}" fill="#cfdae5" font-family="sans-serif" font-size="13" letter-spacing="3">${square ? 'STUDIO SESSIONS' : kind === 'video' ? 'FIELD NOTES' : '2026'}</text>
  </svg>`;
}
const thumbnail = (index) =>
  `data:image/svg+xml,${encodeURIComponent(artwork('video', index))}`;

export async function installDensityData(page) {
  await page.route('**/mock-art/**', (route) => {
    const [, kind, index] = new URL(route.request().url()).pathname
      .split('/')
      .slice(1);
    return route.fulfill({
      contentType: 'image/svg+xml',
      body: artwork(kind, Number(index)),
    });
  });
  await page.route('**/api/v1/catalog?**', (route) => {
    const url = new URL(route.request().url());
    const kind = url.searchParams.get('kind') ?? 'movie';
    const search = (url.searchParams.get('q') ?? '').toLowerCase();
    return route.fulfill({
      json: {
        items: Array.from({ length: 24 }, (_, index) => ({
          id: `${kind}-${index}`,
          kind,
          title: `${titles[index % titles.length]}${index >= 12 ? ' II' : ''}`,
          year: 2026 - (index % 7),
          available: true,
          metadata: {},
          artwork_url: `/mock-art/${kind}/${index}`,
        })).filter((item) => item.title.toLowerCase().includes(search)),
      },
    });
  });
  const now = Math.floor(Date.now() / 1000);
  for (const platform of ['youtube', 'twitch']) {
    await page.route(`**/api/v1/online/${platform}`, (route) =>
      route.fulfill({
        json: {
          configured: true,
          downloads_enabled: false,
          account: {
            status: 'connected',
            display_name: 'Preview viewer',
            external_id: 'preview',
          },
          sync: { last_complete: now, error: null },
        },
      }),
    );
  }
  const videos = Array.from({ length: 24 }, (_, index) => ({
    videoId: `preview${String(index).padStart(4, '0')}`,
    channelId: 'field-notes',
    channelName: 'Field Notes',
    title: `${titles[index % titles.length]} · Episode ${index + 1}`,
    thumbnailUrl: thumbnail(index),
    publishedAt: new Date(now * 1000).toISOString(),
    durationSeconds: 600 + index * 45,
    broadcastState: 'none',
    isLive: false,
    isUpcoming: false,
    isLiveReplay: false,
    positionSeconds: 0,
    watchedPercentage: 0,
    isWatched: false,
    availabilityStatus: 'available',
  }));
  await page.route('**/api/v1/online/youtube/browse', (route) =>
    route.fulfill({
      json: {
        items: videos,
        page: 1,
        pageSize: 60,
        hasMore: false,
        counts: {
          all: videos.length,
          unwatched: videos.length,
          inProgress: 0,
          watched: 0,
          shorts: 0,
          live: 0,
          liveReplays: 0,
          upcoming: 0,
          subscribedChannelCount: 1,
        },
        channels: [{ channelId: 'field-notes', title: 'Field Notes' }],
      },
    }),
  );
  await page.route('**/api/v1/online/youtube/watchlists', (route) =>
    route.fulfill({
      json: [
        {
          id: 1,
          name: 'Watch Later',
          isDefault: true,
          items: [],
          sortMode: 'manual',
        },
      ],
    }),
  );
  await page.route('**/api/v1/online/youtube/home', (route) =>
    route.fulfill({
      json: {
        continue_watching: [],
        next_up: videos.slice(0, 6),
      },
    }),
  );
  await page.route('**/api/v1/online/twitch/feed', (route) =>
    route.fulfill({
      json: {
        items: Array.from({ length: 24 }, (_, index) => ({
          id: `twitch-${index}`,
          login: `channel${index}`,
          display_name: `Channel ${index + 1}`,
          title: titles[index % titles.length],
          category: ['Science & Technology', 'Music', 'Travel'][index % 3],
          viewers: 1200 + index * 137,
          thumbnail_url: thumbnail(index),
          started_at: new Date((now - 3600) * 1000).toISOString(),
          fetched_at: now,
        })),
      },
    }),
  );
  await page.route('**/api/v1/online/kick', (route) =>
    route.fulfill({
      json: {
        connected: true,
        configured: true,
        items: Array.from({ length: 24 }, (_, index) => ({
          slug: `creator${index}`,
          display_name: `Creator ${index + 1}`,
          title: titles[index % titles.length],
          category: ['Just Chatting', 'Music', 'Gaming'][index % 3],
          live: true,
          viewers: 750 + index * 91,
          thumbnail_url: thumbnail(index + 2),
          started_at: new Date((now - 1800) * 1000).toISOString(),
          updated_at: now,
          error: null,
        })),
      },
    }),
  );
}
