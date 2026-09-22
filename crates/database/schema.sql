-- The complete schema for a fresh Thelxinoe database.
-- Database::open creates this in one transaction. There are no upgrade scripts.
-- Times are UTC Unix seconds unless a column explicitly names another unit.
-- History/statistics deliberately retain identity snapshots after media removal.
-- Provider metadata is private per user; downloaded public bytes are shared.

CREATE TABLE users (
    id TEXT NOT NULL PRIMARY KEY,
    username TEXT NOT NULL COLLATE NOCASE UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('admin','user')),
    timezone_override TEXT,
    created_at INTEGER NOT NULL,
    time_format_override TEXT CHECK(time_format_override IN ('12h','24h'))
) STRICT;

CREATE TABLE sessions (
    id TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    transport TEXT NOT NULL CHECK(transport IN ('web','device','jellyfin')),
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    UNIQUE(user_id,id)
) STRICT;

CREATE TABLE secrets (
    scope TEXT NOT NULL PRIMARY KEY,
    ciphertext BLOB NOT NULL
) STRICT;

CREATE TABLE settings (
    key TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE audit (
    id INTEGER PRIMARY KEY,
    actor_id TEXT,
    action TEXT NOT NULL,
    target TEXT NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE jobs (
    id TEXT NOT NULL PRIMARY KEY,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    dedupe_key TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK(state IN ('queued','running','complete','failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at INTEGER NOT NULL,
    started_at INTEGER,
    completed_at INTEGER,
    error TEXT,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE job_effects (
    job_id TEXT NOT NULL PRIMARY KEY REFERENCES jobs(id),
    value TEXT NOT NULL
) STRICT;

CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE playback_grants (
    token_hash TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    resource TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    FOREIGN KEY(user_id,session_id) REFERENCES sessions(user_id,id) ON DELETE CASCADE
) STRICT;

CREATE TABLE login_attempts (
    address TEXT NOT NULL PRIMARY KEY,
    count INTEGER NOT NULL,
    window_start INTEGER NOT NULL
) STRICT;

CREATE TABLE library_roots (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('movies','shows','music')),
    path TEXT NOT NULL UNIQUE,
    automatic_unmanaged_deletion INTEGER NOT NULL DEFAULT 0 CHECK(automatic_unmanaged_deletion IN (0,1)),
    last_scan INTEGER,
    scan_error TEXT
) STRICT;

CREATE TABLE media (
    id TEXT NOT NULL PRIMARY KEY,
    root_id TEXT NOT NULL REFERENCES library_roots(id),
    kind TEXT NOT NULL CHECK(kind IN ('movie','show','season','episode','artist','album','track')),
    parent_id TEXT REFERENCES media(id),
    evidence_key TEXT NOT NULL,
    title TEXT NOT NULL,
    sort_number INTEGER,
    year INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    overrides TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    UNIQUE(root_id,kind,evidence_key)
) STRICT;

CREATE TABLE media_files (
    id TEXT NOT NULL PRIMARY KEY,
    root_id TEXT NOT NULL REFERENCES library_roots(id),
    path TEXT NOT NULL UNIQUE,
    generation TEXT NOT NULL,
    size INTEGER NOT NULL,
    modified TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    probe TEXT NOT NULL,
    edition TEXT NOT NULL DEFAULT '',
    present INTEGER NOT NULL DEFAULT 1 CHECK(present IN (0,1)),
    ownership TEXT NOT NULL DEFAULT 'unresolved' CHECK(ownership IN ('managed','unmanaged','unresolved')),
    scanned_at INTEGER NOT NULL
) STRICT;

CREATE TABLE media_sources (
    media_id TEXT NOT NULL REFERENCES media(id),
    file_id TEXT NOT NULL REFERENCES media_files(id),
    PRIMARY KEY(media_id,file_id)
) STRICT;

CREATE TABLE local_trailers (
    media_id TEXT NOT NULL REFERENCES media(id),
    file_id TEXT NOT NULL REFERENCES media_files(id),
    PRIMARY KEY(media_id,file_id)
) STRICT;

CREATE TABLE media_state (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    watched INTEGER NOT NULL DEFAULT 0 CHECK(watched IN (0,1)),
    updated_at INTEGER NOT NULL,
    favorite INTEGER NOT NULL DEFAULT 0 CHECK(favorite IN (0,1)),
    watch_later INTEGER NOT NULL DEFAULT 0 CHECK(watch_later IN (0,1)),
    watched_revision INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(user_id, media_id)
) STRICT;

CREATE TABLE edition_progress (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    edition TEXT NOT NULL,
    position REAL NOT NULL,
    duration REAL NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(user_id, media_id, edition)
) STRICT;

CREATE TABLE playback_preferences (
    user_id TEXT NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE playlists (
    id TEXT NOT NULL PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    revision INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE playlist_items (
    playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    PRIMARY KEY(playlist_id, position)
) STRICT;

CREATE TABLE playlist_favorites (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    PRIMARY KEY(user_id, playlist_id)
) STRICT;

CREATE TABLE music_queues (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    items TEXT NOT NULL,
    current_index INTEGER NOT NULL DEFAULT 0,
    position REAL NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0 CHECK(completed IN (0,1)),
    PRIMARY KEY(user_id, client_id)
) STRICT;

CREATE TABLE compat_devices (
    session_id TEXT NOT NULL PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL,
    client TEXT NOT NULL,
    version TEXT NOT NULL
) STRICT;

CREATE TABLE compat_preferences (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client TEXT NOT NULL,
    preference_id TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY(user_id,client,preference_id)
) STRICT;

CREATE TABLE quick_connect (
    secret_hash TEXT NOT NULL PRIMARY KEY,
    code_hash TEXT NOT NULL UNIQUE,
    device_id TEXT NOT NULL,
    device_name TEXT NOT NULL,
    client TEXT NOT NULL,
    version TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    authorizer_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE online_accounts (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL CHECK(provider IN ('youtube','twitch','kick')),
    generation TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'disconnected' CHECK(status IN ('disconnected','connected','reconnect_required')),
    display_name TEXT NOT NULL DEFAULT '',
    external_id TEXT NOT NULL DEFAULT '',
    credential BLOB,
    expires_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    avatar_url TEXT,
    profile_checked_at INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(user_id,provider)
) STRICT;

CREATE TABLE oauth_attempts (
    state_hash TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    generation TEXT NOT NULL,
    browser_hash TEXT NOT NULL,
    verifier BLOB NOT NULL,
    redirect_uri TEXT NOT NULL,
    client_hash TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    FOREIGN KEY(user_id,session_id) REFERENCES sessions(user_id,id) ON DELETE CASCADE
) STRICT;

CREATE TABLE youtube_quota (
    day TEXT NOT NULL PRIMARY KEY,
    used INTEGER NOT NULL DEFAULT 0 CHECK(used>=0),
    blocked INTEGER NOT NULL DEFAULT 0 CHECK(blocked IN (0,1))
) STRICT;

CREATE TABLE youtube_quota_users (
    day TEXT NOT NULL REFERENCES youtube_quota(day) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    used INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(day,user_id)
) STRICT;

CREATE TABLE youtube_sync (
    user_id TEXT NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    generation TEXT NOT NULL,
    cursor TEXT NOT NULL DEFAULT '{}',
    next_run INTEGER NOT NULL DEFAULT 0,
    last_turn INTEGER NOT NULL DEFAULT 0,
    last_complete INTEGER,
    failures INTEGER NOT NULL DEFAULT 0,
    error TEXT
) STRICT;

CREATE TABLE youtube_subscriptions (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL,
    title TEXT NOT NULL,
    uploads TEXT,
    snapshot TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0 CHECK(active IN (0,1)),
    thumbnail_url TEXT,
    PRIMARY KEY(user_id,channel_id)
) STRICT;

CREATE TABLE youtube_videos (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    video_id TEXT NOT NULL,
    channel_id TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL,
    channel_title TEXT NOT NULL DEFAULT '',
    published_at INTEGER NOT NULL DEFAULT 0,
    duration INTEGER,
    broadcast TEXT NOT NULL DEFAULT 'none' CHECK(broadcast IN ('none','live','upcoming','replay')),
    privacy TEXT NOT NULL DEFAULT 'unknown',
    available INTEGER NOT NULL DEFAULT 1 CHECK(available IN (0,1)),
    is_short INTEGER CHECK(is_short IN (0,1)),
    short_checked INTEGER NOT NULL DEFAULT 0,
    metadata_at INTEGER NOT NULL DEFAULT 0,
    scheduled_start TEXT,
    actual_start TEXT,
    actual_end TEXT,
    PRIMARY KEY(user_id,video_id)
) STRICT;

CREATE TABLE youtube_state (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    video_id TEXT NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1)),
    watched INTEGER NOT NULL DEFAULT 0 CHECK(watched IN (0,1)),
    position REAL NOT NULL DEFAULT 0 CHECK(position>=0),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(user_id,video_id),
    FOREIGN KEY(user_id,video_id) REFERENCES youtube_videos(user_id,video_id) ON DELETE CASCADE
) STRICT;

CREATE TABLE youtube_downloads (
    video_id TEXT NOT NULL PRIMARY KEY,
    generation TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('queued','downloading','ready','extractor_authentication_required','unavailable','failed','deleting')),
    tools TEXT NOT NULL,
    path TEXT,
    size INTEGER,
    modified TEXT,
    probe TEXT,
    error TEXT,
    requested_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    unprotected_at INTEGER,
    downloaded_bytes INTEGER NOT NULL DEFAULT 0,
    total_bytes INTEGER,
    eta_seconds INTEGER,
    media_kind TEXT
) STRICT;

CREATE TABLE youtube_media (
    video_id TEXT NOT NULL PRIMARY KEY
) STRICT;

CREATE TABLE twitch_attempts (
    user_id TEXT NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    generation TEXT NOT NULL,
    client_hash TEXT NOT NULL,
    device BLOB NOT NULL,
    expires_at INTEGER NOT NULL,
    next_poll INTEGER NOT NULL,
    interval INTEGER NOT NULL,
    error TEXT,
    FOREIGN KEY(user_id,session_id) REFERENCES sessions(user_id,id) ON DELETE CASCADE
) STRICT;

CREATE TABLE twitch_sync (
    user_id TEXT NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    generation TEXT NOT NULL,
    next_run INTEGER NOT NULL DEFAULT 0,
    last_turn INTEGER NOT NULL DEFAULT 0,
    validated_at INTEGER NOT NULL DEFAULT 0,
    last_complete INTEGER,
    cursor TEXT NOT NULL DEFAULT '',
    snapshot TEXT NOT NULL DEFAULT '',
    pages INTEGER NOT NULL DEFAULT 0,
    failures INTEGER NOT NULL DEFAULT 0,
    error TEXT
) STRICT;

CREATE TABLE twitch_streams (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL,
    login TEXT NOT NULL,
    display_name TEXT NOT NULL,
    title TEXT NOT NULL,
    category TEXT NOT NULL,
    viewers INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    snapshot TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0 CHECK(active IN (0,1)),
    thumbnail_url TEXT,
    profile_image_url TEXT,
    PRIMARY KEY(user_id,channel_id)
) STRICT;

CREATE TABLE live_media (
    id TEXT NOT NULL PRIMARY KEY,
    title TEXT NOT NULL
) STRICT;

CREATE TABLE playback_sessions (
    id TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    auth_session_id TEXT NOT NULL,
    media_id TEXT REFERENCES media(id) ON DELETE CASCADE,
    file_id TEXT REFERENCES media_files(id),
    generation TEXT NOT NULL,
    edition TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('ready','playing','paused','stopped','failed')),
    mode TEXT NOT NULL CHECK(mode IN ('direct','remux','transcode')),
    options TEXT NOT NULL,
    duration REAL NOT NULL,
    position REAL NOT NULL DEFAULT 0,
    sequence INTEGER NOT NULL DEFAULT -1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    client_id TEXT,
    queue_revision INTEGER,
    queue_index INTEGER,
    youtube_video_id TEXT REFERENCES youtube_media(video_id),
    streaming INTEGER NOT NULL DEFAULT 0 CHECK(streaming IN (0,1)),
    live_media_id TEXT REFERENCES live_media(id),
    reported_at_ms INTEGER,
    client_active_seconds REAL,
    CHECK((media_id IS NOT NULL AND file_id IS NOT NULL AND youtube_video_id IS NULL AND live_media_id IS NULL) OR (media_id IS NULL AND file_id IS NULL AND youtube_video_id IS NOT NULL AND live_media_id IS NULL) OR (media_id IS NULL AND file_id IS NULL AND youtube_video_id IS NULL AND live_media_id IS NOT NULL AND streaming=1)),
    FOREIGN KEY(user_id,auth_session_id) REFERENCES sessions(user_id,id) ON DELETE CASCADE
) STRICT;

CREATE TABLE compat_playbacks (
    playback_id TEXT NOT NULL PRIMARY KEY REFERENCES playback_sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE compat_audio_playbacks (
    auth_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    playback_id TEXT NOT NULL REFERENCES playback_sessions(id) ON DELETE CASCADE,
    PRIMARY KEY(auth_session_id,media_id)
) STRICT;

CREATE TABLE kick_channels (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    slug TEXT NOT NULL,
    generation TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    category TEXT NOT NULL DEFAULT '',
    live INTEGER CHECK(live IN (0,1)),
    viewers INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    next_run INTEGER NOT NULL DEFAULT 0,
    last_turn INTEGER NOT NULL DEFAULT 0,
    failures INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    thumbnail_url TEXT,
    started_at TEXT,
    display_name TEXT,
    profile_image_url TEXT,
    language TEXT,
    mature INTEGER NOT NULL DEFAULT 0 CHECK(mature IN (0,1)),
    tags TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY(user_id,slug)
) STRICT;

CREATE TABLE manager_services (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('radarr','sonarr','lidarr')),
    container_id TEXT NOT NULL UNIQUE,
    port INTEGER NOT NULL,
    generation TEXT NOT NULL,
    credential BLOB NOT NULL,
    media_source TEXT NOT NULL,
    defaults TEXT NOT NULL DEFAULT '{}',
    version TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
    checked_at INTEGER NOT NULL,
    error TEXT
) STRICT;

CREATE TABLE acquisition_users (
    user_id TEXT NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    auto_approve INTEGER NOT NULL DEFAULT 0 CHECK(auto_approve IN (0,1))
) STRICT;

CREATE TABLE acquisition_requests (
    id TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    service_id TEXT NOT NULL REFERENCES manager_services(id),
    generation TEXT NOT NULL,
    external_id TEXT NOT NULL,
    title TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending','approved','adding','searching','requested','available','failed','denied','cancelled','uncertain')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    manager_id INTEGER,
    error TEXT
) STRICT;

CREATE TABLE manager_bindings (
    file_id TEXT NOT NULL REFERENCES media_files(id),
    service_id TEXT NOT NULL REFERENCES manager_services(id),
    generation TEXT NOT NULL,
    service_generation TEXT NOT NULL,
    manager_file_id INTEGER NOT NULL,
    entity_id INTEGER NOT NULL,
    manager_path TEXT NOT NULL,
    external_id TEXT NOT NULL,
    members TEXT NOT NULL,
    checked_at INTEGER NOT NULL,
    PRIMARY KEY(file_id,service_id)
) STRICT;

CREATE TABLE manager_reconciliations (
    service_id TEXT NOT NULL PRIMARY KEY REFERENCES manager_services(id),
    generation TEXT NOT NULL,
    checked_at INTEGER NOT NULL,
    error TEXT
) STRICT;

CREATE TABLE media_operations (
    id TEXT NOT NULL PRIMARY KEY,
    actor_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    media_id TEXT NOT NULL REFERENCES media(id),
    action TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending','executing','complete','blocked','uncertain')),
    targets TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    error TEXT
) STRICT;

CREATE TABLE media_protection (
    media_id TEXT NOT NULL PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    keep INTEGER NOT NULL DEFAULT 0 CHECK(keep IN (0,1))
) STRICT;

CREATE TABLE support_services (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('bazarr','prowlarr','nzbget')),
    container_id TEXT NOT NULL UNIQUE,
    port INTEGER NOT NULL,
    generation TEXT NOT NULL,
    credential BLOB NOT NULL,
    media_source TEXT NOT NULL,
    native_url TEXT NOT NULL DEFAULT '',
    version TEXT NOT NULL,
    checked_at INTEGER NOT NULL,
    error TEXT
) STRICT;

CREATE TABLE stack_provisions (
    id TEXT NOT NULL PRIMARY KEY,
    kind TEXT NOT NULL UNIQUE,
    actor_id TEXT NOT NULL REFERENCES users(id),
    host_port INTEGER NOT NULL,
    credential BLOB NOT NULL,
    state TEXT NOT NULL,
    container_id TEXT,
    service_id TEXT,
    error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    origin TEXT NOT NULL DEFAULT 'installed' CHECK(origin IN ('installed','adopted')),
    native_url TEXT NOT NULL DEFAULT ''
) STRICT;

CREATE TABLE service_update_policy (
    service_id TEXT NOT NULL PRIMARY KEY,
    policy TEXT NOT NULL CHECK(policy IN ('automatic','notify','manual','inherit')),
    window_start INTEGER NOT NULL DEFAULT 0 CHECK(window_start BETWEEN 0 AND 23),
    window_end INTEGER NOT NULL DEFAULT 0 CHECK(window_end BETWEEN 0 AND 23),
    checked_at INTEGER NOT NULL DEFAULT 0,
    candidate TEXT,
    error TEXT
) STRICT;

CREATE TABLE service_updates (
    id TEXT NOT NULL PRIMARY KEY,
    service_id TEXT NOT NULL,
    actor_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    state TEXT NOT NULL,
    candidate TEXT,
    error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    automatic INTEGER NOT NULL DEFAULT 0 CHECK(automatic IN (0,1))
) STRICT;

CREATE TABLE retention_policies (
    domain TEXT NOT NULL PRIMARY KEY CHECK(domain IN ('movies','shows')),
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
    grace_seconds INTEGER NOT NULL DEFAULT 604800 CHECK(grace_seconds BETWEEN 0 AND 31536000),
    exclude_specials INTEGER NOT NULL DEFAULT 1 CHECK(exclude_specials IN (0,1)),
    trigger_users TEXT NOT NULL DEFAULT '[]',
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE retention_candidates (
    id TEXT NOT NULL PRIMARY KEY,
    media_id TEXT NOT NULL REFERENCES media(id),
    operation_id TEXT REFERENCES media_operations(id),
    stamp TEXT NOT NULL,
    trigger_user TEXT REFERENCES users(id) ON DELETE SET NULL,
    state TEXT NOT NULL CHECK(state IN ('pending','executing','complete','cancelled','blocked')),
    eligible_at INTEGER NOT NULL,
    due_at INTEGER NOT NULL,
    error TEXT,
    UNIQUE(media_id,stamp)
) STRICT;

CREATE TABLE retention_exclusions (
    service_id TEXT NOT NULL REFERENCES manager_services(id),
    external_id TEXT NOT NULL,
    exclusion_id INTEGER NOT NULL,
    created_here INTEGER NOT NULL CHECK(created_here IN (0,1)),
    PRIMARY KEY(service_id,external_id)
) STRICT;

CREATE TABLE media_segments (
    id TEXT NOT NULL PRIMARY KEY,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    generation TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('Intro','Recap','Credits','Preview')),
    start REAL NOT NULL CHECK(start>=0),
    end REAL NOT NULL CHECK(end>start),
    source TEXT NOT NULL CHECK(source IN ('manual','local','theintrodb')),
    confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1),
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE segment_overrides (
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    generation TEXT NOT NULL,
    PRIMARY KEY(media_id,file_id,generation)
) STRICT;

CREATE TABLE segment_preferences (
    user_id TEXT NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE segment_fingerprints (
    file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    generation TEXT NOT NULL,
    window TEXT NOT NULL,
    offset REAL NOT NULL,
    hashes TEXT NOT NULL,
    PRIMARY KEY(file_id,generation,window)
) STRICT;

CREATE TABLE segment_analysis (
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    generation TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('queued','running','complete','failed')),
    requested_at INTEGER NOT NULL,
    completed_at INTEGER,
    error TEXT,
    PRIMARY KEY(media_id,file_id,generation)
) STRICT;

CREATE TABLE notifications (
    id TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    severity TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    read_at INTEGER,
    UNIQUE(user_id,source)
) STRICT;

CREATE TABLE notification_conditions (
    source TEXT NOT NULL PRIMARY KEY,
    occurrence INTEGER NOT NULL,
    active INTEGER NOT NULL CHECK(active IN (0,1))
) STRICT;

CREATE TABLE ui_preferences (
    user_id TEXT NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE youtube_watchlists (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0 CHECK(is_default IN (0,1)),
    auto_download INTEGER NOT NULL DEFAULT 0 CHECK(auto_download IN (0,1)),
    auto_remove_watched INTEGER NOT NULL DEFAULT 0 CHECK(auto_remove_watched IN (0,1)),
    sort_mode TEXT NOT NULL DEFAULT 'manual' CHECK(sort_mode IN ('manual','date')),
    sort_direction TEXT NOT NULL DEFAULT 'asc' CHECK(sort_direction IN ('asc','desc')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(user_id,id)
) STRICT;

CREATE TABLE youtube_watchlist_items (
    user_id TEXT NOT NULL,
    watchlist_id INTEGER NOT NULL,
    video_id TEXT NOT NULL,
    manual_position REAL NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY(watchlist_id,video_id),
    FOREIGN KEY(user_id,watchlist_id) REFERENCES youtube_watchlists(user_id,id) ON DELETE CASCADE,
    FOREIGN KEY(user_id,video_id) REFERENCES youtube_videos(user_id,video_id) ON DELETE CASCADE
) STRICT;

CREATE TABLE youtube_download_suppressed (
    video_id TEXT NOT NULL PRIMARY KEY
) STRICT;

CREATE TABLE compat_online_items (
    id TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    youtube_video_id TEXT,
    twitch_channel_id TEXT,
    kick_slug TEXT,
    watchlist_id INTEGER,
    favorite INTEGER NOT NULL DEFAULT 0 CHECK(favorite IN (0,1)),
    CHECK ((youtube_video_id IS NOT NULL) + (twitch_channel_id IS NOT NULL) + (kick_slug IS NOT NULL) + (watchlist_id IS NOT NULL) = 1),
    UNIQUE(user_id,youtube_video_id),
    UNIQUE(user_id,twitch_channel_id),
    UNIQUE(user_id,kick_slug),
    UNIQUE(user_id,watchlist_id),
    FOREIGN KEY(user_id,youtube_video_id) REFERENCES youtube_videos(user_id,video_id) ON DELETE CASCADE,
    FOREIGN KEY(user_id,twitch_channel_id) REFERENCES twitch_streams(user_id,channel_id) ON DELETE CASCADE,
    FOREIGN KEY(user_id,kick_slug) REFERENCES kick_channels(user_id,slug) ON DELETE CASCADE,
    FOREIGN KEY(user_id,watchlist_id) REFERENCES youtube_watchlists(user_id,id) ON DELETE CASCADE
) STRICT;

CREATE TABLE user_avatars (
    user_id TEXT NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    image TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE playback_statistics (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    platform TEXT NOT NULL CHECK(platform IN ('youtube','twitch','kick','movies','shows','music')),
    media_id TEXT NOT NULL,
    media_title TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    channel_name TEXT NOT NULL,
    category_name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    first_played_at INTEGER NOT NULL,
    last_played_at INTEGER NOT NULL,
    duration REAL NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0 CHECK(completed IN (0,1)),
    PRIMARY KEY(user_id,platform,media_id)
) STRICT;

CREATE TABLE playback_activity (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    bucket_started_at INTEGER NOT NULL,
    platform TEXT NOT NULL CHECK(platform IN ('youtube','twitch','kick','movies','shows','music')),
    media_id TEXT NOT NULL,
    active_seconds REAL NOT NULL CHECK(active_seconds>=0),
    media_title TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    channel_name TEXT NOT NULL,
    category_name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    PRIMARY KEY(user_id,bucket_started_at,platform,media_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE metadata_bindings (
    media_id TEXT NOT NULL PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    service_id TEXT NOT NULL REFERENCES manager_services(id),
    service_generation TEXT NOT NULL,
    external_id TEXT NOT NULL,
    manager_entity_id INTEGER,
    refreshed_at INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE manager_episodes (
    service_id TEXT NOT NULL REFERENCES manager_services(id),
    service_generation TEXT NOT NULL,
    series_external_id TEXT NOT NULL,
    manager_episode_id INTEGER NOT NULL,
    season_number INTEGER NOT NULL,
    episode_number INTEGER NOT NULL,
    refreshed_at INTEGER NOT NULL,
    metadata TEXT NOT NULL,
    PRIMARY KEY(service_id,service_generation,manager_episode_id)
) STRICT;

CREATE TABLE manager_episode_mappings (
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    service_id TEXT NOT NULL,
    service_generation TEXT NOT NULL,
    manager_episode_id INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('confirmed','unresolved','complex')),
    PRIMARY KEY(media_id,service_id,service_generation,manager_episode_id),
    FOREIGN KEY(service_id,service_generation,manager_episode_id) REFERENCES manager_episodes(service_id,service_generation,manager_episode_id) ON DELETE CASCADE
) STRICT;

CREATE TABLE playback_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    playback_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    platform TEXT NOT NULL CHECK(platform IN ('youtube','twitch','kick','movies','shows','music')),
    media_id TEXT NOT NULL,
    media_title TEXT NOT NULL,
    content_type TEXT NOT NULL,
    edition TEXT NOT NULL DEFAULT '',
    device_name TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    ended_at INTEGER,
    position REAL NOT NULL,
    duration REAL NOT NULL DEFAULT 0,
    played_seconds REAL NOT NULL DEFAULT 0,
    state TEXT NOT NULL CHECK(state IN ('playing','paused','stopped')),
    UNIQUE(playback_id,platform)
) STRICT;

CREATE INDEX sessions_user ON sessions(user_id);

CREATE INDEX jobs_ready ON jobs(state, available_at);

CREATE INDEX media_parent ON media(parent_id);

CREATE INDEX media_state_lists ON media_state(user_id, favorite, watch_later);

CREATE INDEX oauth_expiry ON oauth_attempts(expires_at);

CREATE INDEX youtube_sync_ready ON youtube_sync(next_run,last_turn);

CREATE INDEX youtube_feed ON youtube_videos(user_id,published_at DESC);

CREATE INDEX playback_active ON playback_sessions(state,updated_at);

CREATE INDEX playback_youtube ON playback_sessions(youtube_video_id,state,updated_at);

CREATE INDEX kick_ready ON kick_channels(next_run,last_turn);

CREATE UNIQUE INDEX acquisition_own_active ON acquisition_requests(user_id,service_id,external_id) WHERE state NOT IN ('denied','cancelled','failed');

CREATE UNIQUE INDEX retention_pending_media ON retention_candidates(media_id) WHERE state IN ('pending','executing');

CREATE INDEX media_segments_source ON media_segments(media_id,file_id,generation,source);

CREATE INDEX notification_user ON notifications(user_id,created_at DESC);

CREATE UNIQUE INDEX youtube_default_watchlist ON youtube_watchlists(user_id) WHERE is_default=1;

CREATE INDEX playback_activity_date ON playback_activity(bucket_started_at,platform);

CREATE UNIQUE INDEX manager_services_kind ON manager_services(kind);

CREATE UNIQUE INDEX support_services_kind ON support_services(kind);

CREATE INDEX playback_history_time ON playback_history(started_at DESC,id DESC);

CREATE INDEX playback_history_user_platform_time
 ON playback_history(user_id,platform,started_at DESC,id DESC);

CREATE VIEW media_cards AS
SELECT m.id,m.kind,
    COALESCE(json_extract(m.overrides,'$.title'),json_extract(m.metadata,'$.title'),json_extract(m.metadata,'$.name'),m.title) AS title,
    m.parent_id,m.sort_number,m.year,
    EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1) AS available
FROM media m;

CREATE VIEW statistics_metadata AS
SELECT NULL AS user_id, m.id AS media_id,
       CASE m.kind WHEN 'movie' THEN 'movies' WHEN 'episode' THEN 'shows' ELSE 'music' END AS platform,
       m.title AS media_title,
       COALESCE(grandparent.id,parent.id,m.id) AS channel_id,
       COALESCE(grandparent.title,parent.title,m.title) AS channel_name,
       CASE WHEN m.kind='track' THEN COALESCE(parent.title,'') ELSE '' END AS category_name,
       m.kind AS content_type
FROM media_cards m
LEFT JOIN media_cards parent ON parent.id=m.parent_id
LEFT JOIN media_cards grandparent ON grandparent.id=parent.parent_id
WHERE m.kind IN ('movie','episode','track')
UNION ALL
SELECT user_id,'youtube:'||video_id,'youtube',title,channel_id,channel_title,'',
       CASE WHEN broadcast='live' THEN 'live' WHEN broadcast='upcoming' THEN 'upcoming'
            WHEN is_short=1 THEN 'short' WHEN broadcast='replay' THEN 'live_replay' ELSE 'upload' END
FROM youtube_videos
UNION ALL
SELECT user_id,'twitch:'||channel_id,'twitch',title,channel_id,display_name,category,'live'
FROM twitch_streams
UNION ALL
SELECT user_id,'kick:'||slug,'kick',title,slug,slug,category,'live'
FROM kick_channels;

CREATE VIEW user_profiles AS
WITH defaults AS (
    SELECT
        COALESCE((SELECT value FROM settings WHERE key='timezone'),'UTC') AS server_timezone,
        COALESCE((SELECT value FROM settings WHERE key='time_format'),'24h') AS server_time_format
)
SELECT u.id,u.username,u.role,
    COALESCE(u.timezone_override,d.server_timezone) AS timezone,
    u.timezone_override,d.server_timezone,
    COALESCE(u.time_format_override,d.server_time_format) AS time_format,
    u.time_format_override,d.server_time_format
FROM users u CROSS JOIN defaults d;

CREATE TRIGGER retention_watched_revision AFTER UPDATE OF watched ON media_state
WHEN OLD.watched != NEW.watched BEGIN
 UPDATE media_state SET watched_revision=OLD.watched_revision+1 WHERE user_id=NEW.user_id AND media_id=NEW.media_id;
END;

CREATE TRIGGER users_keep_admin_delete BEFORE DELETE ON users
WHEN OLD.role='admin' AND (SELECT COUNT(*) FROM users WHERE role='admin')<=1 BEGIN
 SELECT RAISE(ABORT,'Keep at least one administrator');
END;

CREATE TRIGGER users_keep_admin_role BEFORE UPDATE OF role ON users
WHEN OLD.role='admin' AND NEW.role<>'admin' AND (SELECT COUNT(*) FROM users WHERE role='admin')<=1 BEGIN
 SELECT RAISE(ABORT,'Keep at least one administrator');
END;

CREATE TRIGGER statistics_media_watched_insert AFTER INSERT ON media_state BEGIN
    UPDATE playback_statistics SET completed=NEW.watched WHERE user_id=NEW.user_id AND media_id=NEW.media_id;
END;

CREATE TRIGGER statistics_media_watched_update AFTER UPDATE OF watched ON media_state BEGIN
    UPDATE playback_statistics SET completed=NEW.watched WHERE user_id=NEW.user_id AND media_id=NEW.media_id;
END;

CREATE TRIGGER statistics_youtube_watched_insert AFTER INSERT ON youtube_state BEGIN
    UPDATE playback_statistics SET completed=NEW.watched WHERE user_id=NEW.user_id AND media_id='youtube:'||NEW.video_id;
END;

CREATE TRIGGER statistics_youtube_watched_update AFTER UPDATE OF watched ON youtube_state BEGIN
    UPDATE playback_statistics SET completed=NEW.watched WHERE user_id=NEW.user_id AND media_id='youtube:'||NEW.video_id;
END;

-- Membership and its timestamp have one owner: youtube_watchlist_items.
-- This read model also supplies defaults before a user interacts with a video.
CREATE VIEW youtube_video_state AS
SELECT v.user_id,v.video_id,
    EXISTS(SELECT 1 FROM youtube_watchlist_items i
           WHERE i.user_id=v.user_id AND i.video_id=v.video_id) AS watchlist,
    (SELECT MIN(i.added_at) FROM youtube_watchlist_items i
     WHERE i.user_id=v.user_id AND i.video_id=v.video_id) AS added_at,
    COALESCE(s.pinned,0) AS pinned,COALESCE(s.watched,0) AS watched,
    COALESCE(s.position,0) AS position
FROM youtube_videos v LEFT JOIN youtube_state s USING(user_id,video_id);

CREATE INDEX youtube_watchlist_video ON youtube_watchlist_items(user_id,video_id,added_at);

CREATE INDEX youtube_video_id ON youtube_videos(video_id,user_id);

CREATE INDEX playback_auth_session ON playback_sessions(user_id,auth_session_id);

-- Defaults required before any administrator saves settings.
INSERT INTO settings(key,value) VALUES ('segments.config','{"local":true,"external":false}');
INSERT INTO retention_policies(domain,updated_at) VALUES ('movies',0),('shows',0);
