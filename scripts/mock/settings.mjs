import { admin, definitions, label, now } from './data.mjs';
import { checkConnection, connectionFailure, stagesApi } from './transfers.mjs';

export function settingsSeed() {
  const services = definitions
    .filter(([, , role]) => role !== 'job')
    .map(([kind, port, role]) => ({
      id: `${role}-${kind}`,
      kind,
      name: label(kind),
      container_id: `container-${kind}`,
      port,
      url_base: kind === 'nzbget' ? '' : `/services/${kind}`,
      access_url: `/services/${kind}`,
      native_url: '',
      version: {
        seerr: '3.2.0',
        radarr: '6.0.0',
        sonarr: '4.0.0',
        lidarr: '3.0.0',
        bazarr: '1.5.0',
        prowlarr: '2.0.0',
        nzbget: '25.0',
      }[kind],
      checked_at: now(),
      error: null,
      ...(role === 'manager'
        ? {
            defaults: {
              root_folder: `/media/${kind === 'radarr' ? 'movies' : kind === 'sonarr' ? 'shows' : 'music'}`,
              quality_profile: 1,
              metadata_profile: kind === 'lidarr' ? 1 : null,
              monitored: true,
            },
            retention_enabled: true,
          }
        : {}),
    }));
  const stack = definitions.map(([kind, port, role]) => ({
    id: `managed-${kind}`,
    kind,
    name: label(kind),
    registered: true,
    phase: 'active',
    image: `mock/${kind}:stable`,
    drift: false,
    running: true,
    existence: 'present',
    status: 'running',
    inspection_error: null,
    can_recreate: true,
    can_retire: true,
    can_remove: true,
    error: null,
    transfer_pending: false,
    origin: 'installed',
    provision: {
      id: `managed-${kind}`,
      kind,
      state: 'complete',
      host_port: port ? port + 10000 : null,
      container_id: `container-${kind}`,
      service_id: role === 'job' ? null : `${role}-${kind}`,
      error: null,
      origin: 'installed',
      hold: null,
      transfer_id: null,
    },
  }));
  const defaults = {
    image: 'mock/recyclarr:stable',
    resources: 'preview',
    files: {
      'recyclarr.yml':
        'radarr:\n  movies:\n    base_url: !secret radarr_url\n    api_key: !secret radarr_api_key\n    quality_profiles:\n      - name: HD-1080p\n',
      'settings.yml': '{}\n',
    },
  };
  return {
    services,
    stack,
    connections: [
      ...['radarr', 'sonarr', 'lidarr'].map((kind) => [
        kind,
        'nzbget',
        'download_client',
      ]),
      ...['radarr', 'sonarr', 'lidarr'].map((kind) => [
        'prowlarr',
        kind,
        'application',
      ]),
      ...['radarr', 'sonarr'].flatMap((kind) => [
        ['seerr', kind, 'manager'],
        ['bazarr', kind, 'manager'],
      ]),
    ].map(([source, target, kind], i) => ({
      id: `connection-${i}`,
      source_id: `${['radarr', 'sonarr', 'lidarr'].includes(source) ? 'manager' : 'support'}-${source}`,
      target_id: `${target === 'nzbget' ? 'support' : 'manager'}-${target}`,
      source_kind: source,
      target_kind: target,
      target_name: label(target),
      kind,
      enabled: true,
      state: 'connected',
      error: null,
      cleanup_pending: false,
    })),
    profiles: Object.fromEntries(
      services
        .filter((s) => s.defaults)
        .map((s) => [
          s.id,
          [
            { id: 1, name: 'HD-1080p' },
            { id: 2, name: 'UHD-2160p' },
          ],
        ]),
    ),
    updates: {
      policies: stack.map((item) => ({
        service_id: item.id,
        policy: 'inherit',
        window_start: 3,
        window_end: 5,
        candidate: item.kind === 'radarr' ? 'mock/radarr:6.1.0' : null,
        release:
          item.kind === 'radarr'
            ? { version: '6.1.0', source: 'image', notes_url: null }
            : null,
        checked_at: now(),
        error: null,
      })),
      items: [],
      services: stack.map((item) => ({ id: item.id, kind: item.kind })),
      timezone: 'Europe/Paris',
      server_policy: { policy: 'notify', window_start: 3, window_end: 5 },
    },
    product: {
      version: '0.1.0',
      timezone: 'Europe/Paris',
      configured: true,
      policy: { policy: 'notify', window_start: 3, window_end: 5 },
      release: { version: '0.1.1', notes: 'Preview release' },
      observation: { checked_at: now(), error: null },
      controller: { items: [] },
    },
    tools: ['yt-dlp', 'deno', 'streamlink', 'ffmpeg'].map((id, i) => ({
      id,
      policy: 'inherit',
      effective_policy: 'notify',
      channel: i === 0 ? 'nightly' : 'stable',
      pinned: false,
      revision: 1,
      installed: {
        id: `${id}-installed`,
        candidate_id: `${id}-installed`,
        version: ['2026.09.30', '2.5.0', '7.6.0', '8.0'][i],
      },
      previous: {
        id: `${id}-previous`,
        version: ['2026.09.20', '2.4.0', '7.5.0', '7.1'][i],
      },
      candidate: {
        id: `${id}-candidate`,
        version: ['2026.10.01', '2.5.1', '7.6.1', '8.0.1'][i],
        notes_url: null,
      },
      checked_at: now(),
      check_error: null,
      integrity_error: null,
      job: null,
    })),
    serverSettings: {
      timezone: 'Europe/Paris',
      time_format: '24h',
      video_transcoding: true,
      audio_transcoding: true,
    },
    youtubeConfig: {
      google_configured: true,
      redirect_uri: null,
      youtube_downloads: true,
      youtube_daily_quota: 10000,
      youtube_network: 'auto',
      youtube_network_status: { ipv4: true, ipv6: true, current: 'ipv6' },
      quota: { used: 1420, blocked: false },
    },
    providers: { youtube: true, twitch: true, kick: true },
    providerConfig: {
      youtube: { configured: true },
      twitch: { configured: true },
      kick: { configured: true },
    },
    playbackPreferences: {
      quality: 'auto',
      audio_language: 'eng',
      subtitle_language: 'eng',
      subtitles: false,
      replay_gain: 'track',
    },
    segments: {
      config: { local: true, external: true },
      items: [
        {
          media_id: 'episode-0',
          title: 'Beyond the Horizon · First Contact',
          state: 'complete',
          error: null,
        },
        {
          media_id: 'episode-1',
          title: 'Beyond the Horizon · The Crossing',
          state: 'pending',
          error: null,
        },
      ],
    },
    skipping: { Intro: 'Ask', Recap: 'Ask', Credits: 'Ask', Preview: 'Ignore' },
    retention: {
      policies: ['movies', 'shows', 'videos'].map((domain) => ({
        domain,
        enabled: domain === 'videos',
        grace_seconds: 604800,
        exclude_specials: true,
        trigger_users: [admin.id],
        storage_limit_bytes: domain === 'videos' ? 50 * 1024 ** 3 : null,
      })),
      video_usage: {
        unpinned_bytes: 8 * 1024 ** 3,
        pinned_bytes: 2 * 1024 ** 3,
        waiting: 0,
      },
      items: [
        {
          id: 'retention-1',
          title: 'Parallel Worlds',
          state: 'pending',
          reason: 'watched',
          due_at: now() + 86400,
          eligible_at: now() - 3600,
          watched_users: [admin.username],
          error: null,
        },
        {
          id: 'retention-2',
          title: 'Open Water',
          state: 'complete',
          reason: 'watched',
          due_at: now() - 86400,
          eligible_at: now() - 172800,
          watched_users: [admin.username],
          error: null,
        },
      ],
    },
    backups: {
      destination: '/state/backups',
      policy: { policy: 'automatic', retain: 7 },
      passphrase_saved: true,
      automatic: { state: 'complete', updated_at: now() - 86400, error: null },
      window: { start: 3, end: 5 },
      items: [0, 1, 2].map((i) => ({
        id: `backup-${i}`,
        stage: 'complete',
        created_at: now() - (i + 1) * 86400,
        error: null,
        archive: `thelxinoe-preview-${i}.tar.age`,
        automatic: i !== 2,
      })),
    },
    jobs: [
      'Library scan',
      'YouTube synchronization',
      'Segment analysis',
      'Database checkpoint',
    ].map((kind, i) => ({
      id: `job-${i}`,
      kind,
      state: i === 2 ? 'running' : 'complete',
      error: null,
      created_at: now() - i * 600,
    })),
    sessions: [
      {
        id: 'session-0',
        name: 'This browser',
        transport: 'web',
        last_seen: now(),
      },
      {
        id: 'session-1',
        name: 'Living room desktop',
        transport: 'desktop',
        last_seen: now() - 3600,
      },
      {
        id: 'session-2',
        name: 'Phone',
        transport: 'web',
        last_seen: now() - 7200,
      },
    ],
    auth: {
      password: true,
      totp: false,
      passkeys: [
        {
          id: 'passkey-1',
          name: 'Security key',
          created_at: now() - 86400 * 10,
        },
      ],
      oidc: null,
      fresh: true,
    },
    clientPasswords: [
      {
        id: 'client-password-1',
        name: 'TV client',
        created_at: now() - 86400 * 4,
        last_used_at: now() - 3600,
      },
    ],
    audit: [
      'user.create',
      'settings.update',
      'manager.register',
      'backup.create',
      'request.auto_approve',
      'session.revoke',
    ].map((action, i) => ({
      id: `audit-${i}`,
      action,
      target: [
        'Alex',
        'Display defaults',
        'Radarr',
        'Automatic backup',
        'Northern Lights',
        'Chrome',
      ][i],
      actor: admin.username,
      actor_id: admin.id,
      created_at: now() - i * 2400,
    })),
    oidc: {
      provider: {
        issuer: 'https://identity.example.test',
        client_id: 'preview',
        label: 'Preview identity',
        secret_saved: true,
      },
      redirect_uri: null,
    },
    recyclarr: {
      settings: { provision_id: 'managed-recyclarr', paused: false, hour: 3 },
      timezone: 'Europe/Paris',
      runs: [
        {
          id: 'recyclarr-run-1',
          state: 'complete',
          created_at: now() - 86400,
          started_at: now() - 86400,
          ended_at: now() - 86340,
          error: null,
        },
      ],
    },
    configuration: {
      revision: 'preview-configuration-1',
      mode: 'defaults',
      image: defaults.image,
      files: { ...defaults.files },
      defaults,
      base_defaults: defaults,
      bindings: ['radarr', 'sonarr'].map((kind) => ({
        kind,
        instance: kind,
        service_id: `manager-${kind}`,
        base_url: `${kind}_url`,
        api_key: `${kind}_api_key`,
      })),
      candidate: null,
    },
    supportSnapshots: {
      'support-prowlarr': {
        health: [],
        indexers: [
          {
            id: 1,
            name: 'Northstar Usenet',
            enabled: true,
            disabled_until: null,
          },
          { id: 2, name: 'Horizon Index', enabled: true, disabled_until: null },
        ],
      },
      'support-nzbget': {
        rate: 12000,
        limit: 0,
        paused: false,
        free_mb: 102400,
        queue: [
          {
            id: 1,
            title: 'Beyond the Horizon',
            status: 'DOWNLOADING',
            size_mb: 8192,
            remaining_mb: 2048,
          },
          {
            id: 2,
            title: 'Northern Lights S02',
            status: 'PAUSED',
            size_mb: 4096,
            remaining_mb: 1024,
          },
        ],
        history: [
          {
            id: 100,
            title: 'Nightfall',
            status: 'SUCCESS/ALL',
            size_mb: 4096,
            downloaded_mb: 4096,
            remaining_mb: 0,
            history_time: now() - 3600,
          },
        ],
      },
      'support-bazarr': { health: [], queue: [], history: [] },
      'support-seerr': { health: [] },
    },
  };
}

export async function settingsApi(s, c) {
  const { path, method, body, json, fail } = c;
  const save = (value) => {
    if (method !== 'GET') Object.assign(value, body);
    return json(value);
  };
  if (path === '/online/providers') return json(s.providers);
  if (path.startsWith('/admin/online/providers/')) {
    s.providers[path.split('/').at(-1)] = body.enabled;
    c.emit('providers.changed', s.providers);
    return json(s.providers);
  }
  if (path === '/admin/online') {
    if (method === 'PUT') {
      Object.assign(s.youtubeConfig, body);
      if (body.google) s.youtubeConfig.google_configured = true;
    }
    return json({
      ...s.youtubeConfig,
      redirect_uri: `${c.origin}/api/v1/online/youtube/callback`,
    });
  }
  if (/^\/admin\/online\/(youtube|twitch|kick)$/.test(path))
    return save(s.providerConfig[path.split('/').at(-1)]);
  if (path === '/me/appearance') {
    if (method === 'PATCH') {
      const previous = s.appearance.provider_preferences;
      Object.assign(s.appearance, body);
      s.appearance.provider_preferences = {
        ...previous,
        ...body.provider_preferences,
      };
      c.emit('appearance.changed', { appearance: s.appearance });
    }
    return json(s.appearance);
  }
  if (path === '/me/preferences') {
    if (method === 'PUT')
      Object.assign(s.preferences, {
        timezone_override: body.timezone ?? null,
        time_format_override: body.time_format ?? null,
      });
    return json({
      ...s.preferences,
      timezone: s.preferences.timezone_override ?? s.serverSettings.timezone,
      time_format:
        s.preferences.time_format_override ?? s.serverSettings.time_format,
      server_timezone: s.serverSettings.timezone,
      server_time_format: s.serverSettings.time_format,
    });
  }
  if (path === '/admin/settings') return save(s.serverSettings);
  if (path === '/admin/audit') {
    const action = c.query.get('action')?.split(',');
    const user = c.query.get('user');
    return json({
      items: s.audit.filter(
        (item) =>
          (!action || action.includes(item.action)) &&
          (!user || item.actor_id === user),
      ),
      next_before: null,
      actors: s.users,
      actions: [...new Set(s.audit.map((item) => item.action))],
    });
  }
  if (path === '/playback/preferences') return save(s.playbackPreferences);
  if (path === '/me/segments') return save(s.skipping);
  if (path === '/admin/segments') {
    if (method === 'PUT') Object.assign(s.segments.config, body);
    return json(s.segments);
  }
  if (path === '/admin/jobs') {
    if (method === 'POST')
      s.jobs.unshift({
        id: `job-${++s.sequence}`,
        kind: 'Database checkpoint',
        state: 'complete',
        error: null,
        created_at: now(),
      });
    return json({ items: s.jobs });
  }
  if (path === '/admin/health')
    return json({
      version: s.product.version,
      controller: true,
      cache_free_bytes: 80 * 1024 ** 3,
    });
  if (path === '/admin/operations')
    return json({
      storage: {
        state: {
          bytes: 780 * 1024 ** 2,
          free_bytes: 80 * 1024 ** 3,
          partial: false,
        },
        cache: {
          bytes: 12 * 1024 ** 3,
          free_bytes: 80 * 1024 ** 3,
          partial: false,
        },
      },
      support: {
        items: [
          {
            id: 'support-nzbget',
            kind: 'nzbget',
            problem: false,
            unavailable: false,
            rate: 12000,
            paused: s.supportSnapshots['support-nzbget'].paused,
            remaining_mb: 3072,
            queue_count: 2,
          },
          {
            id: 'support-prowlarr',
            kind: 'prowlarr',
            problem: false,
            health_count: 0,
          },
        ],
        checked_at: now(),
      },
      playback: [
        {
          id: 'preview-playback',
          user: 'Alex',
          title: 'Northern Lights',
          mode: 'direct',
          state: 'paused',
        },
      ],
      errors: [],
      services: s.services.map((item) => ({
        kind: item.kind,
        version: item.version,
        healthy: !item.error,
        checked_at: now(),
      })),
    });
  if (path === '/admin/devices')
    return json({
      items: [
        {
          id: 'device-1',
          name: 'Living room',
          user: admin.username,
          last_seen: now() - 3600,
          transport: 'desktop',
        },
      ],
    });
  if (path === '/admin/diagnostics' || path === '/admin/operations/diagnostics')
    return json({ preview: true, services: s.services, jobs: s.jobs });
  if (path === '/admin/tools') return json({ supported: true, items: s.tools });
  if (path === '/admin/tools/check') {
    s.tools.forEach((item) => (item.checked_at = now()));
    return json({ queued: true });
  }
  const toolRoute = path.match(
    /^\/admin\/tools\/([^/]+)\/(settings|install|rollback|cancel)$/,
  );
  if (toolRoute) {
    const tool = s.tools.find((item) => item.id === toolRoute[1]);
    if (!tool) return fail(404, 'Tool not found');
    if (toolRoute[2] === 'settings') {
      Object.assign(tool, body);
      tool.revision++;
      tool.effective_policy =
        tool.policy === 'inherit' ? s.product.policy.policy : tool.policy;
    }
    if (toolRoute[2] === 'install') {
      tool.previous = tool.installed;
      tool.installed = tool.candidate;
      tool.candidate = null;
    }
    if (toolRoute[2] === 'rollback')
      [tool.installed, tool.previous] = [tool.previous, tool.installed];
    c.emit('tools.changed', {});
    return json(tool);
  }
  if (path === '/admin/product-update') return json(s.product);
  if (path.startsWith('/admin/product-update/')) {
    const action = path.split('/').at(-1);
    if (action === 'policy') {
      Object.assign(s.product.policy, body);
      Object.assign(s.updates.server_policy, body);
    }
    if (action === 'check') s.product.observation.checked_at = now();
    if (action === 'install') {
      s.product.version = s.product.release.version;
      s.product.release = null;
      s.product.request = null;
    }
    c.emit('product.update', {});
    return json({ saved: true });
  }
  if (path === '/admin/retention')
    return json({ ...s.retention, users: s.users });
  const policy = path.match(/^\/admin\/retention\/policy\/([^/]+)$/);
  if (policy)
    return save(s.retention.policies.find((p) => p.domain === policy[1]));
  const retention = path.match(
    /^\/admin\/retention\/([^/]+)\/(keep|delete|cancel|retry)$/,
  );
  if (retention) {
    const item = s.retention.items.find((item) => item.id === retention[1]);
    if (!item) return fail(404, 'Retention entry not found');
    item.state = {
      keep: 'kept',
      cancel: 'cancelled',
      retry: 'pending',
      delete: 'complete',
    }[retention[2]];
    return json({ saved: true });
  }
  if (path === '/admin/backups') {
    if (method === 'POST')
      s.backups.items.unshift({
        id: `backup-${++s.sequence}`,
        stage: 'complete',
        archive: `thelxinoe-preview-${s.sequence}.tar.age`,
        created_at: now(),
        automatic: false,
        error: null,
      });
    return json(s.backups);
  }
  if (path === '/admin/backups/policy') return save(s.backups.policy);
  if (path === '/admin/backups/passphrase') {
    s.backups.passphrase_saved = true;
    return json({ saved: true });
  }
  if (/^\/admin\/backups\/[^/]+\/restore$/.test(path))
    return json({ queued: true });
  if (path === '/users') {
    if (method === 'POST') {
      const user = {
        id: `user-${++s.sequence}`,
        username: body.username,
        role: body.role ?? 'user',
        timezone: 'Europe/Paris',
      };
      s.users.push(user);
      return json({ user });
    }
    return json({ items: s.users });
  }
  const userRoute = path.match(/^\/users\/([^/]+)(\/recovery)?$/);
  if (userRoute) {
    const user = s.users.find((item) => item.id === userRoute[1]);
    if (!user) return fail(404, 'User not found');
    if (userRoute[2]) return json({ url: `${c.origin}/#settings/account` });
    if (method === 'PUT') Object.assign(user, body);
    if (method === 'DELETE') s.users = s.users.filter((item) => item !== user);
    return json({ user });
  }
  if (path === '/auth/sessions')
    return json({ current: 'session-0', items: s.sessions });
  if (path.startsWith('/auth/sessions/') && method === 'DELETE') {
    s.sessions = s.sessions.filter(
      (item) => item.id !== path.split('/').at(-1),
    );
    return json({ deleted: true });
  }
  if (path === '/auth/devices')
    return json({
      items: s.sessions.filter((item) => item.transport === 'desktop'),
    });
  if (path === '/me/auth') return json(s.auth);
  if (path === '/me/auth/passkeys/register/finish') {
    s.auth.passkeys.push({
      id: `passkey-${++s.sequence}`,
      name: body.name,
      created_at: now(),
    });
    return json({ saved: true });
  }
  if (path === '/me/auth/client-passwords') {
    if (method === 'POST') {
      const item = {
        id: `client-password-${++s.sequence}`,
        name: body.name,
        created_at: now(),
        password: 'preview-client-password',
      };
      s.clientPasswords.push(item);
      return json(item);
    }
    return json({ items: s.clientPasswords });
  }
  if (path.startsWith('/me/auth/client-passwords/') && method === 'DELETE') {
    s.clientPasswords = s.clientPasswords.filter(
      (item) => item.id !== path.split('/').at(-1),
    );
    return json({ deleted: true });
  }
  if (path === '/me/password') {
    s.auth.password = true;
    return json({ saved: true });
  }
  if (path === '/me/auth/totp/start')
    return json({
      secret: 'JBSWY3DPEHPK3PXP',
      uri: `otpauth://totp/Preview:admin?secret=JBSWY3DPEHPK3PXP&issuer=Preview`,
      issuer: 'Preview',
      account: admin.username,
      digits: 6,
      period: 30,
      algorithm: 'SHA1',
    });
  if (path === '/me/auth/totp/confirm') {
    s.auth.totp = true;
    return json({ enabled: true });
  }
  if (path.startsWith('/me/auth/') && method === 'DELETE') {
    const name = path.split('/')[3];
    if (name === 'passkeys')
      s.auth.passkeys = s.auth.passkeys.filter(
        (item) => item.id !== path.split('/').at(-1),
      );
    else s.auth[name] = name === 'oidc' ? null : false;
    return json({ deleted: true });
  }
  if (path === '/me/avatar') {
    s.users[0].avatar = body.image ?? null;
    c.emit('user.changed', s.users[0]);
    return json({ avatar: s.users[0].avatar });
  }
  if (path === '/admin/auth/oidc') {
    if (method === 'DELETE') s.oidc.provider = null;
    if (method === 'POST') s.oidc.provider = { ...body, secret_saved: true };
    return json({
      ...s.oidc,
      redirect_uri: `${c.origin}/api/v1/auth/oidc/callback`,
    });
  }
  return false;
}

export async function servicesApi(s, c) {
  const { path, method, body, json, fail } = c;
  const staged = await stagesApi(s, c);
  if (staged) return staged;
  if (path === '/admin/managers' || path === '/admin/support') {
    const manager = path.endsWith('managers');
    if (method === 'POST') {
      const error = await connectionFailure(s, body.kind);
      if (error) return fail(422, error);
      const role = manager ? 'manager' : 'support';
      const item = {
        ...body,
        id: `${role}-${body.kind}`,
        version: '1.0.0',
        checked_at: now(),
        error: null,
        defaults: {},
        access_url: `/services/${body.kind}`,
      };
      s.services = s.services.filter((service) => service.kind !== body.kind);
      s.services.push(item);
      return json(item);
    }
    return json({
      items: s.services.filter((service) => !!service.defaults === manager),
    });
  }
  if (path === '/admin/managers/containers')
    return json({
      items: definitions.map(([kind, port]) => ({
        id: `container-${kind}`,
        names: [`/${kind}`],
        image: `mock/${kind}:stable`,
        state: 'running',
        ports: port
          ? [
              {
                Type: 'tcp',
                PrivatePort: port,
                PublicPort: port + 10000,
                IP: '127.0.0.1',
              },
            ]
          : [],
      })),
    });
  if (path === '/admin/managers/reconcile') return json({ queued: true });
  if (path === '/admin/managers/bindings')
    return json({
      items: s.library
        .filter((item) => item.kind === 'movie')
        .slice(0, 4)
        .map((item, i) => ({
          id: item.fileId,
          path: `/media/movies/${item.title}.mp4`,
          ownership: 'confirmed',
          bindings: [
            {
              service: 'Radarr',
              file_id: i + 1,
              entity_id: i + 1,
              members: [],
            },
          ],
        })),
    });
  if (path === '/admin/stack')
    return json({
      items: s.stack,
      provisions: s.stack.map((item) => item.provision),
    });
  if (path === '/admin/service-connections') {
    if (method === 'POST') {
      const connection = s.connections.find(
        (item) =>
          item.source_id === body.source_id &&
          item.target_id === body.target_id,
      );
      if (!connection) return fail(404, 'Connection not found');
      const config = s.scenarios?.services[connection.source_kind];
      if (['unavailable', 'pending', 'cleanup'].includes(config?.linkState))
        return json(connection);
      if (
        config?.linkState === 'conflict' &&
        body.action !== 'import' &&
        body.action !== 'disconnect'
      )
        return fail(
          409,
          'Import the existing record or remove it in the service first.',
        );
      connection.enabled = body.action !== 'disconnect';
      connection.state = connection.enabled ? 'connected' : 'available';
      connection.error = null;
      connection.candidate = null;
      connection.cleanup_pending = false;
      return json(connection);
    }
    return json({
      items: s.connections.filter((link) =>
        [link.source_id, link.target_id].every((id) =>
          s.services.some((service) => service.id === id),
        ),
      ),
    });
  }
  if (path.startsWith('/admin/service-connections/')) {
    const id = path.split('/').at(-1);
    let connection = s.connections.find((item) => item.id === id);
    if (!connection) {
      connection = {
        id,
        ...body,
        state: 'connected',
        error: null,
        checked_at: now(),
      };
      s.connections.push(connection);
    }
    if (method === 'PUT') Object.assign(connection, body);
    return json(connection);
  }
  if (path === '/admin/acquisition/users')
    return json({
      items: s.users.map((user) => ({
        ...user,
        enabled: s.approvals[user.id] ?? user.role === 'admin',
      })),
    });
  if (path.startsWith('/admin/acquisition/users/') && method === 'PUT') {
    s.approvals[path.split('/').at(-1)] = body.enabled;
    return json({ saved: true });
  }
  const serviceRoute = path.match(
    /^\/admin\/(managers|support|services)\/([^/]+)(?:\/(.*))?$/,
  );
  if (serviceRoute) {
    const [, , id, action] = serviceRoute;
    const service = s.services.find((item) => item.id === id);
    if (!service) return fail(404, 'Service not found');
    if (action === 'options')
      return json({
        defaults: service.defaults,
        roots: [{ id: 1, path: service.defaults.root_folder }],
        profiles: s.profiles[id],
        metadata_profiles: [{ id: 1, name: 'Standard' }],
      });
    if (action === 'defaults') {
      Object.assign(service.defaults, body);
      return json({ saved: true });
    }
    if (action === 'retention') {
      service.retention_enabled = body.enabled;
      return json({ saved: true });
    }
    if (action === 'native-url') {
      service.native_url = body.url ?? body.native_url;
      return json({ saved: true });
    }
    if (action === 'connection/test')
      return checkConnection(s, c, service, false);
    if (action === 'connection') {
      if (method === 'PUT') {
        const error = await connectionFailure(s, service.kind);
        if (error) return fail(422, error);
        Object.assign(service, body, { error: null, checked_at: now() });
      }
      return json({ username: 'preview', ...service });
    }
    if (action === 'quality-profiles') {
      if (method === 'POST') {
        const profile = { id: ++s.sequence, name: body.name };
        s.profiles[id].push(profile);
        service.defaults.quality_profile = profile.id;
        return json(profile);
      }
      return json({
        qualities: [
          { id: 1, name: 'WEB-720p', resolution: 720 },
          { id: 2, name: 'WEB-1080p', resolution: 1080 },
          { id: 3, name: 'Bluray-1080p', resolution: 1080 },
          { id: 4, name: 'WEB-2160p', resolution: 2160 },
        ],
      });
    }
    if (action === 'login')
      return json({ username: 'preview', password: 'preview-only' });
    if (action === 'indexers/schema')
      return json({
        items: [
          {
            name: 'Newznab',
            implementation: 'Newznab',
            protocol: 'usenet',
            privacy: 'private',
            fields: [
              {
                name: 'baseUrl',
                label: 'URL',
                type: 'textbox',
                value: 'https://indexer.example.test',
              },
              { name: 'apiKey', label: 'API key', type: 'password' },
            ],
          },
        ],
        profiles: [{ id: 1, name: 'Standard' }],
      });
    if (action === 'indexers/test') return json({ healthy: true });
    if (action === 'indexers/fields') return json({ options: [] });
    if (action === 'indexers' && method === 'POST') {
      s.supportSnapshots[id].indexers.push({
        id: ++s.sequence,
        name: body.name,
        enabled: true,
        disabled_until: null,
      });
      return json({ saved: true });
    }
    if (!action && s.supportSnapshots[id]) {
      const snapshot = s.supportSnapshots[id];
      if (method === 'POST') {
        if (['pause_all', 'resume_all'].includes(body.action))
          snapshot.paused = body.action === 'pause_all';
        if (['pause', 'resume'].includes(body.action)) {
          const item = snapshot.queue.find((item) => item.id === body.item_id);
          if (item)
            item.status = body.action === 'pause' ? 'PAUSED' : 'DOWNLOADING';
        }
        if (body.action === 'remove')
          snapshot.queue = snapshot.queue.filter(
            (item) => item.id !== body.item_id,
          );
      }
      return json(snapshot);
    }
  }
  if (path === '/admin/service-updates') return json(s.updates);
  const updatePolicy = path.match(
    /^\/admin\/service-updates\/policy\/([^/]+)$/,
  );
  if (updatePolicy) {
    Object.assign(
      s.updates.policies.find((item) => item.service_id === updatePolicy[1]),
      body,
    );
    return json({ saved: true });
  }
  const preflight = path.match(
    /^\/admin\/service-updates\/preflight\/([^/]+)$/,
  );
  if (preflight) {
    const update = {
      id: `update-${++s.sequence}`,
      service_id: preflight[1],
      state: 'ready',
      candidate: s.updates.policies.find(
        (item) => item.service_id === preflight[1],
      )?.candidate,
      error: null,
      classification: 'compatible',
    };
    s.updates.items.unshift(update);
    return json(update);
  }
  const update = path.match(
    /^\/admin\/service-updates\/([^/]+)\/(apply|activate|cancel|rollback|retry)$/,
  );
  if (update) {
    const item = s.updates.items.find((item) => item.id === update[1]);
    if (!item) return fail(404, 'Update not found');
    item.state = update[2] === 'cancel' ? 'cancelled' : 'complete';
    return json(item);
  }
  if (path === '/admin/recyclarr') return json(s.recyclarr);
  if (path === '/admin/recyclarr/schedule') {
    Object.assign(s.recyclarr.settings, body);
    return json({ saved: true });
  }
  if (path === '/admin/recyclarr/sync') {
    const run = {
      id: `recyclarr-run-${++s.sequence}`,
      state: 'complete',
      error: null,
      created_at: now(),
      started_at: now(),
      ended_at: now(),
    };
    s.recyclarr.runs.unshift(run);
    return json(run);
  }
  if (path === '/admin/recyclarr/configuration') {
    if (method === 'PUT') {
      s.configuration.files = body.files;
      s.configuration.mode = 'customized';
      s.configuration.revision = `preview-${++s.sequence}`;
    }
    return json(s.configuration);
  }
  if (path === '/admin/recyclarr/configuration/validate')
    return json({
      valid: true,
      diagnostics: [],
      preview: { preview: 'Preview configuration is valid.' },
    });
  if (path === '/admin/recyclarr/configuration/editor')
    return json({
      version: 'preview',
      revision: s.configuration.revision,
      documents: {
        'config-schema.json': {
          type: 'object',
          properties: {
            radarr: { type: 'object' },
            sonarr: { type: 'object' },
          },
        },
        'settings-schema.json': { type: 'object' },
      },
      catalog: [
        { label: 'HD-1080p', type: 'text' },
        { label: 'UHD-2160p', type: 'text' },
      ],
    });
  if (path === '/admin/recyclarr/configuration/schema')
    return json({ version: 'preview', schemas: {} });
  if (
    path === '/admin/recyclarr/configuration/restore-defaults' ||
    path === '/admin/recyclarr/configuration/defaults'
  ) {
    s.configuration.files = { ...s.configuration.defaults.files };
    s.configuration.mode = 'defaults';
    s.configuration.revision = `preview-${++s.sequence}`;
    return json(s.configuration);
  }
  if (path === '/admin/ownership/reconcile') return json({ queued: true });
  if (path === '/admin/media/reconcile') return json({ queued: true });
  if (path === '/admin/media/operations') {
    if (method === 'GET') return json({ items: Object.values(s.operations) });
    const operation = {
      id: `operation-${++s.sequence}`,
      files: 1,
      media_id: body.media_id,
      title: s.library.find((item) => item.id === body.media_id)?.title,
      action: body.action,
      state: 'pending',
      error: null,
    };
    s.operations[operation.id] = operation;
    return json(operation);
  }
  const mediaOperation = path.match(
    /^\/admin\/media\/operations\/([^/]+)\/execute$/,
  );
  if (mediaOperation) {
    const operation = s.operations[mediaOperation[1]];
    if (!operation) return fail(404, 'Operation not found');
    const item = s.library.find((item) => item.id === operation.media_id);
    if (item && operation.action === 'delete') item.available = false;
    if (item && operation.action !== 'delete')
      item.monitored = operation.action === 'monitor';
    operation.state = 'complete';
    return json({ complete: true });
  }
  const keepMedia = path.match(/^\/admin\/media\/([^/]+)\/keep$/);
  if (keepMedia) {
    const item = s.library.find((item) => item.id === keepMedia[1]);
    if (item) item.keep = body.keep;
    return json({ saved: true });
  }
  if (path === '/admin/stack/install') {
    const seed = settingsSeed();
    const item = seed.stack.find((item) => item.kind === body.kind);
    if (!item) return fail(400, 'Unknown service');
    item.provision.host_port = body.host_port ?? item.provision.host_port;
    s.stack = [
      ...s.stack.filter((service) => service.kind !== body.kind),
      item,
    ];
    const service = seed.services.find((service) => service.kind === body.kind);
    if (service) {
      s.services = [
        ...s.services.filter((item) => item.kind !== body.kind),
        service,
      ];
      if (service.defaults) s.profiles[service.id] = seed.profiles[service.id];
    }
    return json({ id: item.id, queued: true });
  }
  const stackAction = path.match(
    /^\/admin\/stack\/([^/]+)\/(action|retry|login)$/,
  );
  if (stackAction) {
    if (stackAction[2] === 'login')
      return json({ username: 'preview', password: 'preview-only' });
    const item = s.stack.find((item) => item.id === stackAction[1]);
    if (!item) return fail(404, 'Managed service not found');
    if (['remove', 'retire'].includes(body.action)) {
      s.stack = s.stack.filter((service) => service !== item);
      s.services = s.services.filter(
        (service) => service.id !== item.provision.service_id,
      );
    } else {
      item.running = body.action !== 'stop';
      item.status = item.running ? 'running' : 'stopped';
      item.error = null;
      item.provision.state = 'complete';
    }
    return json({ queued: true });
  }
  return false;
}
