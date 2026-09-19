<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { Bookmark, Check, Pin, RefreshCw, Link, Play } from '@lucide/svelte';
  import { api, desktop, serverUrl } from './api';
  import type { MediaChoice } from './playback';
  let { revision = 0, play } = $props<{
    revision?: number;
    play: (choice: MediaChoice) => void;
  }>();
  let downloadStates = $state<Record<string, string>>({});
  async function prepare(video: Video) {
    try {
      const current = await api<{
        enabled: boolean;
        download: { state: string } | null;
      }>(`/online/youtube/videos/${video.id}/download`);
      if (current.download?.state === 'ready') {
        play({ id: `youtube:${video.id}`, title: video.title });
        return;
      }
      if (!current.enabled) {
        notice = 'YouTube downloads are disabled by the administrator.';
        return;
      }
      if (!video.watchlist && !video.pinned) {
        notice = 'Save or pin this video before downloading it.';
        return;
      }
      if (current.download?.state === 'extractor_authentication_required') {
        notice =
          'This video requires extractor authentication. Public-only playback cannot access it.';
        return;
      }
      await api(`/online/youtube/videos/${video.id}/download`, 'POST');
      downloadStates[video.id] = 'queued';
      notice = 'Download queued. Choose Play when it is ready.';
    } catch (e) {
      error = String(e);
    }
  }
  type Account = {
    account: { status: string; display_name: string };
    configured: boolean;
    linking_available: boolean;
    linking_url: string | null;
    quota: { used: number; blocked: boolean };
  };
  type Video = {
    id: string;
    title: string;
    channel: string;
    published_at: number;
    duration: number | null;
    broadcast: string;
    available: boolean;
    is_short: boolean | null;
    pending: boolean;
    watchlist: boolean;
    pinned: boolean;
    watched: boolean;
    position: number;
    artwork_url?: string;
    download?: string;
  };
  type Feed = {
    items: Video[];
    total: number;
    sync: {
      error: string | null;
      last_complete: number | null;
      next_run: number;
      in_progress: boolean;
    } | null;
  };
  let account = $state<Account | null>(null),
    feed = $state<Feed | null>(null),
    error = $state(''),
    notice = $state(''),
    busy = $state(false),
    url = $state(''),
    tab = $state('feed'),
    hideShorts = $state(true),
    unwatched = $state(false),
    search = $state(''),
    offset = $state(0),
    confirmDelete = $state(false);
  let additions = $state<
    { key: string; url: string; id?: string; error?: string }[]
  >([]);
  let request = 0,
    timer: ReturnType<typeof setTimeout> | undefined;
  async function load() {
    const current = ++request;
    const query = new URLSearchParams({
      offset: String(offset),
      watchlist: String(tab === 'watchlist'),
      pinned: String(tab === 'pins'),
      hide_shorts: String(hideShorts),
      unwatched: String(unwatched),
      search,
    });
    try {
      const [a, f] = await Promise.all([
        api<Account>('/online/youtube'),
        api<Feed>(`/online/youtube/feed?${query}`),
      ]);
      if (current !== request) return;
      account = a;
      feed = f;
      for (const video of f.items)
        downloadStates[video.id] = video.download ?? '';
      additions = additions.filter((a) => !a.id);
    } catch (e) {
      if (current === request) error = String(e);
    }
  }
  $effect(() => {
    void revision;
    void tab;
    void hideShorts;
    void unwatched;
    void search;
    void offset;
    untrack(() => {
      clearTimeout(timer);
      timer = setTimeout(() => void load(), 200);
    });
  });
  onMount(() => {
    const result = new URLSearchParams(location.search).get('youtube_link');
    if (result) {
      notice =
        result === 'connected'
          ? 'YouTube connected. Your subscriptions will appear as they synchronize.'
          : 'Account linking did not complete. Try again, or ask the administrator to check the Google Web application and callback address.';
      history.replaceState(null, '', location.pathname);
    }
    return () => {
      request++;
      clearTimeout(timer);
    };
  });
  async function act(action: () => Promise<void>) {
    busy = true;
    error = '';
    try {
      await action();
      await load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function connect() {
    await act(async () => {
      if (desktop) {
        await invoke('open_youtube_linking');
        notice =
          'Continue in your browser. Sign in to Thelxinoe there, then connect YouTube.';
        return;
      }
      if (
        account?.linking_url &&
        new URL(account.linking_url).origin !== location.origin
      ) {
        location.assign(account.linking_url);
        return;
      }
      const result = await api<{ url: string }>(
        '/online/youtube/connect',
        'POST',
      );
      const target = new URL(result.url);
      if (
        target.protocol !== 'https:' ||
        target.hostname !== 'accounts.google.com'
      )
        throw new Error(
          'The server returned an invalid Google authorization address',
        );
      location.assign(target.href);
    });
  }
  async function add() {
    const value = url.trim();
    if (!value) return;
    const key = crypto.randomUUID();
    additions = [{ key, url: value }, ...additions];
    url = '';
    tab = 'watchlist';
    offset = 0;
    try {
      const result = await api<{ id: string }>(
        '/online/youtube/watchlist',
        'POST',
        { url: value },
      );
      additions = additions.map((a) =>
        a.key === key ? { ...a, id: result.id } : a,
      );
      await load();
    } catch (e) {
      additions = additions.map((a) =>
        a.key === key ? { ...a, error: String(e) } : a,
      );
    }
  }
  async function change(video: Video, key: 'watchlist' | 'pinned' | 'watched') {
    await act(async () => {
      await api(`/online/youtube/videos/${video.id}`, 'PUT', {
        [key]: !video[key],
      });
    });
  }
  function duration(seconds: number | null) {
    if (seconds === null) return '';
    const h = Math.floor(seconds / 3600),
      m = Math.floor(seconds / 60) % 60,
      s = seconds % 60;
    return h
      ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
      : `${m}:${String(s).padStart(2, '0')}`;
  }
</script>

<section class="panel">
  <div class="section-heading">
    <div>
      <h2>Your YouTube account</h2>
      <p class="muted">
        {account?.account.status === 'connected'
          ? account.account.display_name
          : account?.account.status === 'reconnect_required'
            ? 'Reconnect to resume synchronization.'
            : 'Connect YouTube to bring in your subscriptions.'}
      </p>
    </div>
    <div class="actions">
      <button
        class="primary"
        disabled={busy || !account?.configured || !account?.linking_available}
        onclick={connect}
        ><Link size={16} />{account?.account.status === 'connected'
          ? 'Reconnect YouTube'
          : 'Connect YouTube'}</button
      >
      {#if account?.account.status === 'connected'}<button
          class="secondary"
          disabled={busy}
          onclick={() =>
            act(async () => {
              await api('/online/youtube', 'DELETE');
              notice =
                'YouTube disconnected. Your saved videos and history remain.';
            })}>Disconnect</button
        >{/if}
    </div>
  </div>
  {#if account && !account.configured}<p class="muted">
      An administrator needs to configure the Google application in Settings.
    </p>{:else if account && !account.linking_available}<p class="muted">
      An administrator needs to configure this server's public URL.
    </p>{/if}
  {#if notice}<p role="status">{notice}</p>{/if}
  <details>
    <summary>Manage my YouTube data</summary>
    <p>
      Disconnecting keeps your feed, watchlist, pins, and history. Deleting your
      YouTube data removes the connection and your saved YouTube data from this
      server.
    </p>
    {#if confirmDelete}<p>Delete all of your YouTube data?</p>
      <button
        class="secondary"
        disabled={busy}
        onclick={() =>
          act(async () => {
            await api('/online/youtube/data', 'DELETE');
            additions = [];
            confirmDelete = false;
            notice = 'Your YouTube data has been deleted.';
          })}>Delete my YouTube data</button
      >
      <button class="secondary" onclick={() => (confirmDelete = false)}
        >Cancel</button
      >
    {:else}<button class="secondary" onclick={() => (confirmDelete = true)}
        >Delete YouTube data…</button
      >{/if}
  </details>
</section>
{#if error}<p class="error" role="alert">{error}</p>{/if}
<section class="panel">
  <form
    class="inline-form"
    onsubmit={(e) => {
      e.preventDefault();
      void add();
    }}
  >
    <label
      >Save a video<input
        bind:value={url}
        placeholder="Paste a YouTube URL"
        aria-label="YouTube video URL"
        maxlength="2048"
      /></label
    ><button class="primary" disabled={!url.trim()}
      ><Bookmark size={16} />Add to watchlist</button
    >
  </form>
</section>
<div class="section-heading">
  <div class="actions" aria-label="YouTube views">
    {#each [['feed', 'Subscriptions'], ['watchlist', 'Watchlist'], ['pins', 'Pinned']] as [value, label] (value)}<button
        class={tab === value ? 'primary' : 'secondary'}
        aria-pressed={tab === value}
        onclick={() => {
          tab = value;
          offset = 0;
        }}>{label}</button
      >{/each}
  </div>
  <button
    class="secondary"
    disabled={busy || account?.account.status !== 'connected'}
    onclick={() =>
      act(async () => {
        await api('/online/youtube/sync', 'POST');
        notice = 'Synchronization queued.';
      })}><RefreshCw size={15} />Sync</button
  >
</div>
<div class="filters">
  <label
    >Search<input
      bind:value={search}
      oninput={() => (offset = 0)}
      placeholder="Title or channel"
    /></label
  >
  <label
    ><input
      type="checkbox"
      bind:checked={hideShorts}
      onchange={() => (offset = 0)}
    />Hide Shorts</label
  >
  <label
    ><input
      type="checkbox"
      bind:checked={unwatched}
      onchange={() => (offset = 0)}
    />Unwatched only</label
  >
</div>
{#if feed?.sync?.error}<p role="status" class="muted">
    {feed.sync.error} Next retry: {new Date(
      feed.sync.next_run * 1000,
    ).toLocaleTimeString()}.
  </p>
{:else if feed?.sync?.in_progress}<p role="status" class="muted">
    Synchronizing your channels…
  </p>
{:else if feed?.sync?.last_complete}<p class="muted">
    Last synchronized {new Date(
      feed.sync.last_complete * 1000,
    ).toLocaleString()}.
  </p>{/if}
{#if tab === 'watchlist'}{#each additions as addition (addition.key)}<div
      class="panel row"
      aria-busy={!addition.id && !addition.error}
    >
      <div>
        <strong>{addition.url}</strong>
        <p class="muted">
          {addition.error ??
            (addition.id
              ? 'Saved — waiting for video details.'
              : 'Adding to your watchlist…')}
        </p>
      </div>
      {#if addition.error}<button
          class="secondary"
          onclick={() =>
            (additions = additions.filter((a) => a.key !== addition.key))}
          >Dismiss</button
        >{/if}
    </div>{/each}{/if}
<div class="youtube-grid">
  {#each feed?.items ?? [] as video (video.id)}
    <article class="video-card">
      <div class="thumbnail">
        {#if video.artwork_url}<img
            src={`${serverUrl()}${video.artwork_url}`}
            alt=""
            loading="lazy"
          />{:else}<Play size={32} />{/if}<span
          >{video.broadcast !== 'none'
            ? video.broadcast
            : duration(video.duration)}</span
        >
      </div>
      <div class="video-body">
        <h3>{video.title}</h3>
        <p class="muted">
          {video.channel || 'Video details pending'}{video.published_at
            ? ` · ${new Date(video.published_at * 1000).toLocaleDateString()}`
            : ''}
        </p>
        {#if video.pending}<p class="muted">
            {account?.account.status === 'connected'
              ? 'Waiting for metadata…'
              : 'Connect YouTube to load details.'}
          </p>{:else if !video.available}<p class="muted">
            This video is unavailable to your connected account.
          </p>{/if}
        {#if video.is_short === true}<span class="badge">Short</span>{/if}
        {#if ['queued', 'downloading'].includes(downloadStates[video.id])}<p
            class="muted"
          >
            Downloading for playback…
          </p>
        {:else if downloadStates[video.id] === 'extractor_authentication_required'}<p
            class="muted"
          >
            Extractor authentication required. This video cannot be played with
            public access.
          </p>
        {:else if ['failed', 'unavailable'].includes(downloadStates[video.id])}<p
            class="muted"
          >
            The public download could not complete. You can retry.
          </p>{/if}
        <div class="actions">
          <button
            class="secondary"
            aria-label={`Play ${video.title}`}
            onclick={() => prepare(video)}
            ><Play size={17} />{downloadStates[video.id] === 'queued'
              ? 'Check download'
              : 'Play'}</button
          >
          <button
            class="secondary"
            title={video.watchlist
              ? 'Remove from watchlist'
              : 'Add to watchlist'}
            aria-label={`${video.watchlist ? 'Remove' : 'Save'} ${video.title} ${video.watchlist ? 'from' : 'to'} watchlist`}
            aria-pressed={video.watchlist}
            disabled={busy}
            onclick={() => change(video, 'watchlist')}
            ><Bookmark size={17} /></button
          >
          <button
            class="secondary"
            title={video.pinned ? 'Unpin' : 'Keep downloaded video'}
            aria-label={`Pin ${video.title}`}
            aria-pressed={video.pinned}
            disabled={busy}
            onclick={() => change(video, 'pinned')}><Pin size={17} /></button
          >
          <button
            class="secondary"
            title={video.watched ? 'Mark unwatched' : 'Mark watched'}
            aria-label={`Watched ${video.title}`}
            aria-pressed={video.watched}
            disabled={busy}
            onclick={() => change(video, 'watched')}><Check size={17} /></button
          >
        </div>
      </div>
    </article>
  {:else}<p class="muted">
      {feed ? 'No videos match this view.' : 'Loading your videos…'}
    </p>{/each}
</div>
{#if feed && feed.total > 50}<div class="section-heading">
    <button
      class="secondary"
      disabled={offset === 0}
      onclick={() => (offset = Math.max(0, offset - 50))}>Previous</button
    ><span
      >{offset + 1}–{Math.min(offset + 50, feed.total)} of {feed.total}</span
    ><button
      class="secondary"
      disabled={offset + 50 >= feed.total}
      onclick={() => (offset += 50)}>Next</button
    >
  </div>{/if}

<style>
  .actions,
  .filters {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
  }
  .filters {
    margin: 1rem 0;
  }
  .actions button {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }
  .actions .secondary[aria-pressed='true'] {
    color: var(--accent);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .youtube-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 1.2rem;
    margin: 1rem 0;
  }
  .filters label:has(input[type='checkbox']) {
    flex-direction: row;
    align-items: center;
    margin-bottom: 0;
  }
  .filters input[type='checkbox'] {
    width: auto;
  }
  .video-card {
    background: var(--surface);
    overflow: hidden;
    border-radius: 10px;
  }
  .thumbnail {
    aspect-ratio: 16/9;
    background: #20303a;
    position: relative;
    display: grid;
    place-items: center;
    color: var(--muted);
  }
  .thumbnail img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    position: absolute;
  }
  .thumbnail span {
    position: absolute;
    right: 8px;
    bottom: 8px;
    background: #10181ee0;
    color: white;
    padding: 2px 6px;
    border-radius: 4px;
    text-transform: capitalize;
  }
  .video-body {
    padding: 1rem;
  }
  .video-body h3 {
    margin: 0;
    font-size: 1rem;
    line-height: 1.5;
  }
  .video-body p {
    font-size: 0.85rem;
  }
  .video-body .actions {
    margin-top: 0.8rem;
  }
  details {
    margin-top: 1rem;
  }
  summary {
    cursor: pointer;
    color: var(--muted);
  }
</style>
