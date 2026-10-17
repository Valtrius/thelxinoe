import { admin, art, iso, now, statistics, viewer } from './data.mjs';

export async function mediaApi(s, c) {
  const { path, method, body, query, json, fail } = c;
  const parents = new Map(s.library.map((item) => [item.id, item.parent_id]));
  const card = (item) => {
    const watched = (entry) => {
      if (!['show', 'season'].includes(entry.kind))
        return Boolean(s.mediaStates[entry.id]?.watched);
      const episodes = s.library.filter(
        (child) =>
          child.kind === 'episode' &&
          (child.parent_id === entry.id ||
            parents.get(child.parent_id) === entry.id),
      );
      return episodes.length > 0 && episodes.every(watched);
    };
    return { ...item, watched: watched(item) };
  };
  if (path === '/catalog') {
    let items = s.library.filter((item) =>
      query.has('parent')
        ? item.parent_id === query.get('parent')
        : item.kind === (query.get('kind') ?? 'movie'),
    );
    if (query.get('q'))
      items = items.filter((item) =>
        item.title.toLowerCase().includes(query.get('q').toLowerCase()),
      );
    if (query.get('collection'))
      items = items.filter(
        (_, index) => index % 3 === Number(query.get('collection').slice(-1)),
      );
    return json({ items: items.map(card) });
  }
  if (path === '/catalog/collections')
    return json({
      items: ['Horizon Collection', 'Night Stories', 'Journeys'].map(
        (name, i) => ({ id: `collection-${i}`, name, count: 8 }),
      ),
    });
  if (path === '/catalog/roots')
    return json({
      media_mount: '/media',
      items: ['movies', 'shows', 'music'].map((kind) => ({
        id: `root-${kind}`,
        name: kind,
        kind,
        path: `/media/${kind}`,
        last_scan: now() - 3600,
        scan_error: null,
      })),
    });
  if (/^\/catalog\/roots\/[^/]+\/scan$/.test(path))
    return json({ id: 'mock-scan', queued: true });
  const catalog = path.match(/^\/catalog\/([^/]+)(?:\/(.*))?$/);
  if (catalog) {
    const item = s.library.find((item) => item.id === catalog[1]);
    if (!item) return fail(404, 'Media not found');
    const action = catalog[2];
    if (action === 'state') {
      s.mediaStates[item.id] ??= {
        favorite: false,
        watch_later: false,
        watched: false,
      };
      if (method === 'PUT') Object.assign(s.mediaStates[item.id], body);
      return json(s.mediaStates[item.id]);
    }
    if (action === 'segments') {
      s.segmentsByMedia[item.id] ??= [
        {
          id: 'intro',
          kind: 'Intro',
          start: 0,
          end: 10,
          source: 'local',
          confidence: 1,
        },
        {
          id: 'credits',
          kind: 'Credits',
          start: 80,
          end: 90,
          source: 'local',
          confidence: 1,
        },
      ];
      if (method === 'PUT')
        s.segmentsByMedia[item.id] = body.reset ? [] : body.items;
      return json({
        generation: 'preview',
        preferences: s.skipping,
        items: s.segmentsByMedia[item.id],
      });
    }
    if (action === 'segments/analyze') return json({ queued: true });
    if (action === 'manager-episodes')
      return json({
        items: s.library
          .filter(
            (episode) =>
              episode.kind === 'episode' &&
              episode.parent_id === item.parent_id,
          )
          .map((episode, i) => ({
            id: episode.id,
            season: item.metadata.season_number,
            episode: i + 1,
            metadata: { name: episode.title },
          })),
        mappings: (item.mappings ?? [item.id]).map((id) => ({
          id,
          state: 'confirmed',
        })),
      });
    if (action === 'episode-mapping') {
      item.mappings = body.episode_ids;
      return json({ saved: true });
    }
    if (['analyze', 'analysis', 'refresh', 'match'].includes(action))
      return json({ queued: true });
    if (['metadata', 'overrides'].includes(action)) {
      item.overrides = { ...body };
      if (body.title) item.title = body.title;
      if (body.overview) item.overview = body.overview;
      if (body.year) item.year = body.year;
      return json({ saved: true });
    }
    if (!action)
      return json({
        ...item,
        files: [
          {
            id: item.fileId,
            edition: item.kind === 'movie' ? 'Theatrical' : '',
            present: true,
            path: `/media/${item.kind}/${item.title}.mp4`,
            probe: {
              format: { duration: String(item.duration) },
              streams: [
                {
                  codec_type: 'video',
                  codec_name: 'h264',
                  width: 1920,
                  height: 1080,
                },
                {
                  codec_type: 'audio',
                  codec_name: 'aac',
                  tags: { language: 'eng' },
                },
              ],
            },
          },
        ],
        local_trailers: [],
      });
  }
  if (path === '/me/home') {
    const cards = (items) =>
      items.map((item) => ({ ...card(item), last_played: now() - 3600 }));
    return json({
      continue_watching: cards(
        s.library.filter(
          (item) =>
            item.position > 0 && ['movie', 'episode'].includes(item.kind),
        ),
      ),
      next_up: cards(
        s.library
          .filter(
            (item) =>
              item.kind === 'episode' &&
              item.metadata.season_number === 1 &&
              item.metadata.episode_number === 2,
          )
          .slice(0, 6),
      ),
      watch_later: cards(
        s.library.filter((item) => s.mediaStates[item.id]?.watch_later),
      ),
      favorites: cards(
        s.library.filter((item) => s.mediaStates[item.id]?.favorite),
      ),
    });
  }
  if (path === '/me/statistics' || path === '/admin/statistics')
    return json(statistics(query, s.users));
  if (path === '/me/history' || path === '/admin/history')
    return json({
      items: s.library
        .filter((item) => ['movie', 'episode', 'track'].includes(item.kind))
        .slice(0, 18)
        .map((item, i) => ({
          id: `history-${i}`,
          media_id: item.id,
          kind: item.kind,
          title: item.title,
          edition: '',
          user_id: viewer(s).id,
          username: viewer(s).username,
          device: i % 2 ? 'Living room' : 'Chrome',
          started_at: now() - (i + 1) * 3600,
          updated_at: now() - i * 3600,
          ended_at: i === 0 ? null : now() - i * 3600,
          position: item.duration * 0.7,
          duration: item.duration,
          played_seconds: item.duration * 0.7,
          state: i === 0 ? 'playing' : 'ended',
        })),
      next_before: null,
    });
  if (path === '/playlists') {
    if (method === 'POST') {
      const id = `playlist-${++s.sequence}`;
      s.playlists.push({
        id,
        name: body.name,
        description: body.description ?? '',
        owner_id: viewer(s).id,
        owner: viewer(s).username,
        revision: 1,
        favorite: false,
        items: (body.items ?? [])
          .map((id) => s.library.find((item) => item.id === id))
          .filter(Boolean),
      });
      return json(s.playlists.at(-1));
    }
    return json({
      items: s.playlists.map((p) => ({ ...p, count: p.items.length })),
    });
  }
  const playlist = path.match(/^\/playlists\/([^/]+)(?:\/(favorite))?$/);
  if (playlist) {
    const p = s.playlists.find((p) => p.id === playlist[1]);
    if (!p) return fail(404, 'Playlist not found');
    if (method === 'DELETE') {
      s.playlists = s.playlists.filter((item) => item !== p);
      return json({ deleted: true });
    }
    if (method === 'PUT' || method === 'POST') {
      if (playlist[2]) p.favorite = body.favorite ?? !p.favorite;
      else {
        if (body.name !== undefined) p.name = body.name;
        if (body.description !== undefined) p.description = body.description;
        if (body.items)
          p.items = body.items
            .map((id) => s.library.find((item) => item.id === id))
            .filter(Boolean);
        p.revision++;
      }
    }
    return json({ ...p, count: p.items.length });
  }
  if (path === '/seerr/status') return json({ configured: true, ready: true });
  if (path === '/requests/capabilities')
    return json({ movies: true, tv: true, music: true });
  if (path === '/seerr/requests') {
    if (method === 'POST') {
      const media = s.discover.find(
        (item) =>
          item.mediaType === body.media_type && item.id === body.media_id,
      );
      if (!media) return fail(404, 'Title not found');
      const request = {
        id: ++s.sequence,
        status: 2,
        createdAt: iso(),
        type: media.mediaType,
        requestedBy: {
          id: s.users.indexOf(viewer(s)) + 1,
          username: viewer(s).username,
        },
        seasons: (body.seasons ?? []).map((seasonNumber) => ({
          seasonNumber,
          status: 2,
        })),
        media: { tmdbId: media.id, mediaType: media.mediaType, status: 3 },
      };
      s.requests.unshift(request);
      media.mediaInfo.status = 3;
      media.mediaInfo.requests.push(request);
      media.mediaInfo.seasons.push(
        ...request.seasons.map((season) => ({ ...season, status: 3 })),
      );
      c.emit('request.changed', { id: request.id });
      return json(request);
    }
    const results =
      viewer(s).role === 'admin'
        ? s.requests
        : s.requests.filter(
            (item) => item.requestedBy.username === viewer(s).username,
          );
    return json({
      pageInfo: { pages: 1, page: 1, results: results.length, pageSize: 20 },
      results,
    });
  }
  const request = path.match(
    /^\/seerr\/requests\/(\d+)\/(approve|decline|cancel|retry)$/,
  );
  if (request) {
    const item = s.requests.find((item) => item.id === Number(request[1]));
    if (!item) return fail(404, 'Request not found');
    item.status = request[2] === 'approve' ? 2 : request[2] === 'retry' ? 1 : 3;
    const media = s.discover.find(
      (m) => m.id === item.media.tmdbId && m.mediaType === item.type,
    );
    media.mediaInfo.status = item.status === 2 ? 3 : 1;
    item.media.status = media.mediaInfo.status;
    media.mediaInfo.requests = media.mediaInfo.requests.filter(
      (r) => r.id !== item.id || item.status !== 3,
    );
    return json(item);
  }
  if (path.startsWith('/seerr/profiles/'))
    return json({
      locked: false,
      items: [
        {
          name: 'HD-1080p',
          default: true,
          profile: {
            source: 'custom',
            service_id: path.endsWith('tv')
              ? 'manager-sonarr'
              : 'manager-radarr',
            profile_id: 1,
          },
        },
        {
          name: 'UHD-2160p',
          default: false,
          profile: {
            source: 'custom',
            service_id: path.endsWith('tv')
              ? 'manager-sonarr'
              : 'manager-radarr',
            profile_id: 2,
          },
        },
      ],
    });
  if (path === '/seerr/search' || path.startsWith('/seerr/discover/')) {
    const feed = path.split('/').at(-1);
    let items = s.discover.filter((item) =>
      feed === 'movies' || feed === 'upcoming'
        ? item.mediaType === 'movie'
        : feed === 'tv' || feed === 'upcoming-tv'
          ? item.mediaType === 'tv'
          : true,
    );
    if (query.get('q'))
      items = items.filter((item) =>
        (item.title ?? item.name)
          .toLowerCase()
          .includes(query.get('q').toLowerCase()),
      );
    return json({
      results: items,
      page: Number(query.get('page') ?? 1),
      totalPages: 1,
      totalResults: items.length,
    });
  }
  const detail = path.match(/^\/seerr\/(movie|tv)\/(\d+)(\/recommendations)?$/);
  if (detail) {
    const media = s.discover.find(
      (item) => item.mediaType === detail[1] && item.id === Number(detail[2]),
    );
    if (!media) return fail(404, 'Title not found');
    return json(
      detail[3]
        ? {
            results: s.discover
              .filter((item) => item.mediaType === detail[1] && item !== media)
              .slice(0, 12),
            page: 1,
            totalPages: 1,
            totalResults: 12,
          }
        : media,
    );
  }
  if (path === '/acquisition/requests')
    return json({
      items: [
        {
          id: 'music-request-1',
          title: 'Night Studio · Studio Sessions',
          kind: 'lidarr',
          state: 'available',
        },
        {
          id: 'music-request-2',
          title: 'Open Roads · Night Sessions',
          kind: 'lidarr',
          state: 'searching',
        },
      ],
      page: 1,
      pages: 1,
    });
  if (path === '/metadata/search')
    return json({
      items: s.library
        .filter(
          (item) =>
            item.kind === query.get('kind') &&
            item.title
              .toLowerCase()
              .includes((query.get('q') ?? '').toLowerCase()),
        )
        .slice(0, 8)
        .map((item) => ({
          service_id: 'manager-radarr',
          service_generation: 'preview',
          source: 'tmdb',
          external_id: item.id,
          title: item.title,
          year: String(item.year),
          overview: item.overview,
        })),
    });
  if (path.startsWith('/metadata/')) return json({ saved: true });
  return false;
}

export async function onlineApi(s, c) {
  const { path, method, body, json, fail } = c;
  const platform = path.match(/^\/online\/(youtube|twitch|kick)(?:\/(.*))?$/);
  if (!platform) return false;
  const [, kind, action] = platform;
  if (!action) {
    if (method === 'DELETE') s.connected[kind] = false;
    if (kind === 'kick')
      return json({
        connected: s.connected.kick,
        configured: true,
        items: s.kick.map((item) => ({ ...item, updated_at: now() })),
      });
    return json({
      configured: s.providerConfig[kind]?.configured ?? true,
      linking_available: true,
      linking_url: `${c.origin}/__mock/connect/${kind}`,
      downloads_enabled: s.youtubeConfig.youtube_downloads,
      quota: { blocked: false },
      account: {
        status: s.connected[kind] ? 'connected' : 'disconnected',
        display_name: admin.username,
        external_id: 'preview',
        avatar_url: art('person', 0),
        updated_at: now(),
      },
      sync: {
        last_complete: now(),
        next_run: now() + 60,
        error: null,
        in_progress: false,
      },
    });
  }
  if (['connect', 'sync', 'authorization', 'data'].includes(action)) {
    if (action === 'connect') s.connected[kind] = true;
    if (action === 'data' && method === 'DELETE') {
      if (kind === 'youtube') s.videos = [];
      if (kind === 'kick') s.kick = [];
      if (kind === 'twitch') s.twitch = [];
    }
    c.emit(`${kind}.sync`, { last_complete: now() });
    return json({ url: `${c.origin}/__mock/connect/${kind}`, queued: true });
  }
  if (kind === 'twitch' && action === 'feed')
    return json({
      items: s.twitch.map((item) => ({ ...item, fetched_at: now() })),
    });
  if (kind === 'kick' && action === 'channels' && method === 'POST') {
    const slug = body.channel
      .replace(/^https?:\/\/(www\.)?kick\.com\//, '')
      .replace(/\/$/, '');
    if (!s.kick.some((item) => item.slug === slug))
      s.kick.push({
        slug,
        display_name: slug,
        title: 'Offline',
        category: '',
        live: false,
        viewers: 0,
        profile_image_url: art('person', 2),
        updated_at: now(),
        error: null,
      });
    return json({ slug });
  }
  if (
    kind === 'kick' &&
    action.startsWith('channels/') &&
    method === 'DELETE'
  ) {
    s.kick = s.kick.filter(
      (item) => item.slug !== decodeURIComponent(action.slice(9)),
    );
    return json({ deleted: true });
  }
  if (kind !== 'youtube') return false;
  if (action === 'home')
    return json({
      continue_watching: s.videos.filter(
        (v) => v.positionSeconds > 0 && !v.isWatched,
      ),
      next_up: s.videos
        .filter((v) => !v.isWatched && !v.isUpcoming && !v.isLive)
        .slice(3, 10),
    });
  if (action === 'browse') {
    const q = body?.query ?? {};
    let items = s.videos;
    if (q.search)
      items = items.filter((v) =>
        v.title.toLowerCase().includes(q.search.toLowerCase()),
      );
    if (q.channelId) items = items.filter((v) => v.channelId === q.channelId);
    if (q.watchStates?.length)
      items = items.filter((v) =>
        q.watchStates.includes(
          v.isWatched
            ? 'watched'
            : v.positionSeconds > 0
              ? 'in_progress'
              : 'unwatched',
        ),
      );
    if (!q.includeShorts) items = items.filter((v) => !v.isShort);
    if (!q.includeLive) items = items.filter((v) => !v.isLive);
    if (!q.includeLiveReplays) items = items.filter((v) => !v.isLiveReplay);
    if (!q.includeUpcoming) items = items.filter((v) => !v.isUpcoming);
    if (q.durationFilter === 'under_10')
      items = items.filter((v) => v.durationSeconds < 600);
    if (q.durationFilter === '10_plus')
      items = items.filter((v) => v.durationSeconds >= 600);
    if (q.durationFilter === '30_plus')
      items = items.filter((v) => v.durationSeconds >= 1800);
    if (q.downloadFilter === 'downloaded')
      items = items.filter((v) => v.download?.status === 'ready');
    if (q.publishedFilter === '7_days' || q.publishedFilter === '30_days')
      items = items.filter(
        (v) =>
          Date.parse(v.publishedAt) >=
          (now() - (q.publishedFilter === '7_days' ? 7 : 30) * 86400) * 1000,
      );
    items = [...items].sort(
      (a, b) =>
        (q.sortField === 'channel'
          ? a.channelName.localeCompare(b.channelName)
          : q.sortField === 'duration'
            ? a.durationSeconds - b.durationSeconds
            : Date.parse(a.publishedAt) - Date.parse(b.publishedAt)) *
        (q.sortDirection === 'asc' ? 1 : -1),
    );
    const count = (predicate) => s.videos.filter(predicate).length;
    return json({
      items,
      page: body?.page ?? 1,
      pageSize: body?.pageSize ?? 60,
      hasMore: false,
      counts: {
        all: s.videos.length,
        unwatched: count((v) => !v.isWatched),
        inProgress: count((v) => v.positionSeconds > 0 && !v.isWatched),
        watched: count((v) => v.isWatched),
        shorts: count((v) => v.isShort),
        live: count((v) => v.isLive),
        liveReplays: count((v) => v.isLiveReplay),
        upcoming: count((v) => v.isUpcoming),
        subscribedChannelCount: 3,
      },
      channels: [0, 1, 2].map((i) => ({
        channelId: `channel-${i}`,
        title: ['Field Notes', 'Night Studio', 'Open Roads'][i],
      })),
    });
  }
  if (action === 'resolve') {
    const id = body.video_id ?? body.videoId;
    let video = s.videos.find((v) => v.videoId === id);
    if (!video) {
      video = { ...s.videoTemplate, videoId: id, title: `Added video ${id}` };
      s.videos.unshift(video);
    }
    return json(video);
  }
  if (action === 'watchlists') {
    if (method === 'POST') {
      const id = ++s.sequence;
      s.watchlists.push({
        id,
        name: body.name,
        isDefault: false,
        autoDownload: false,
        autoRemoveWatched: false,
        sortMode: 'manual',
        sortDirection: 'asc',
        createdAt: iso(),
        updatedAt: iso(),
        items: [],
      });
      return json(id);
    }
    return json(s.watchlists);
  }
  const listRoute = action.match(
    /^watchlists\/(\d+)(?:\/(items|order)(?:\/([^/]+))?)?$/,
  );
  if (listRoute) {
    const list = s.watchlists.find((item) => item.id === Number(listRoute[1]));
    if (!list) return fail(404, 'Watchlist not found');
    if (!listRoute[2]) {
      if (method === 'DELETE')
        s.watchlists = s.watchlists.filter((item) => item !== list);
      if (method === 'PUT') Object.assign(list, body, { updatedAt: iso() });
      return json(list);
    }
    if (listRoute[2] === 'order') {
      list.items.sort(
        (a, b) =>
          body.videoIds.indexOf(a.videoId) - body.videoIds.indexOf(b.videoId),
      );
      list.items.forEach((item, i) => (item.manualPosition = i));
      list.updatedAt = iso();
      return json({ saved: true });
    }
    if (method === 'DELETE') {
      list.items = list.items.filter((item) => item.videoId !== listRoute[3]);
      list.updatedAt = iso();
      return json({ deleted: true });
    }
    const video = s.videos.find((v) => v.videoId === body.videoId);
    if (!video) return fail(404, 'Video not found');
    const existing = list.items.find((item) => item.videoId === video.videoId);
    if (!existing) {
      list.items.push({
        videoId: video.videoId,
        video,
        manualPosition: body.manualPosition ?? list.items.length,
        addedAt: iso(),
      });
      list.updatedAt = iso();
      if (list.autoDownload && !video.download)
        video.download = {
          videoId: video.videoId,
          status: 'ready',
          fileSizeBytes: 540 * 1024 ** 2,
          pinned: false,
          quality: '1080p',
          requestedAt: iso(),
        };
    }
    return json({
      video,
      added: !existing,
      manualPosition: body.manualPosition,
    });
  }
  const videoRoute = action.match(/^videos\/([^/]+)(\/download)?$/);
  if (videoRoute) {
    const video = s.videos.find((v) => v.videoId === videoRoute[1]);
    if (!video) return fail(404, 'Video not found');
    if (videoRoute[2]) {
      if (method === 'POST')
        video.download = {
          videoId: video.videoId,
          status: 'ready',
          fileSizeBytes: 540 * 1024 ** 2,
          pinned: false,
          quality: '1080p',
          requestedAt: iso(),
        };
      if (method === 'DELETE') video.download = null;
      return json({
        download: video.download
          ? { state: video.download.status, size: video.download.fileSizeBytes }
          : null,
      });
    }
    if (method === 'PUT') {
      if (body.watched !== undefined) {
        video.isWatched = body.watched;
        video.watchedPercentage = body.watched ? 100 : 0;
        if (body.watched)
          for (const list of s.watchlists)
            if (list.autoRemoveWatched) {
              list.items = list.items.filter(
                (item) => item.videoId !== video.videoId,
              );
              list.updatedAt = iso();
            }
      }
      if (video.download && body.pinned !== undefined)
        video.download.pinned = body.pinned;
    }
    return json(video);
  }
  return false;
}
