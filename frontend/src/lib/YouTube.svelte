<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import {
    Bookmark,
    RefreshCw,
    Link,
    Play,
    PanelRightClose,
    PanelRightOpen,
    X,
  } from '@lucide/svelte';
  import { api, desktop, serverUrl } from './api';
  import YouTubeCard from './ui/YouTubeCard.svelte';
  import FeedGroup from './ui/FeedGroup.svelte';
  import { appearance } from './appearance';
  import { scaleYoutubeMediaScope } from './card-grid-zoom';
  import { type Video, duration } from './youtube-types';
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
        notice = 'This video is already downloaded.';
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
    confirmDelete = $state(false),
    dockOpen = $state(true),
    compact = $state(false),
    watchlist = $state<Feed | null>(null);
  let additions = $state<
    { key: string; url: string; id?: string; error?: string }[]
  >([]);
  let request = 0,
    timer: ReturnType<typeof setTimeout> | undefined;
  const groups = $derived.by(() => {
    const values: { key: string; title: string; items: Video[] }[] = [];
    for (const video of feed?.items ?? []) {
      const date = video.published_at
        ? new Date(video.published_at * 1000)
        : null;
      const key = date?.toLocaleDateString() ?? 'pending';
      let group = values.at(-1);
      if (!group || group.key !== key) {
        group = {
          key,
          title:
            date?.toLocaleDateString(undefined, {
              weekday: 'long',
              day: 'numeric',
              month: 'long',
              year: 'numeric',
            }) ?? 'Video details pending',
          items: [],
        };
        values.push(group);
      }
      group.items.push(video);
    }
    return values;
  });
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
      const [a, f, w] = await Promise.all([
        api<Account>('/online/youtube'),
        api<Feed>(`/online/youtube/feed?${query}`),
        api<Feed>('/online/youtube/feed?watchlist=true'),
      ]);
      if (current !== request) return;
      account = a;
      feed = f;
      watchlist = w;
      for (const video of [...f.items, ...w.items])
        downloadStates[video.id] = video.download ?? '';
      additions = additions.filter(
        (a) => !a.id || !w.items.some((v) => v.id === a.id),
      );
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
    const media = matchMedia('(max-width: 900px)');
    const resize = () => (compact = media.matches);
    resize();
    media.addEventListener('change', resize);
    const result = new URLSearchParams(location.search).get('youtube_link');
    if (result) {
      notice =
        result === 'connected'
          ? 'YouTube connected. Your subscriptions will appear as they synchronize.'
          : 'Account linking did not complete. Try again, or ask the administrator to check the Google Web application and callback address.';
      history.replaceState(null, '', location.pathname);
    }
    return () => {
      media.removeEventListener('change', resize);
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
    dockOpen = true;
    if (compact) tab = 'watchlist';
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
</script>

<details
  class="provider-account"
  open={account?.account.status !== 'connected'}
>
  <summary
    >{account?.account.status === 'connected'
      ? 'Connected as ' + account.account.display_name
      : 'Your YouTube account'}</summary
  >
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
</details>
{#if error}<p class="error" role="alert">{error}</p>{/if}

<div class="view-toolbar">
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
  {#if !compact}<button
      class="icon-button dock-toggle"
      aria-label={dockOpen ? 'Hide watchlist' : 'Show watchlist'}
      title={dockOpen ? 'Hide watchlist' : 'Show watchlist'}
      onclick={() => (dockOpen = !dockOpen)}
      >{#if dockOpen}<PanelRightClose size={18} />{:else}<PanelRightOpen
          size={18}
        />{/if}</button
    >{/if}
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
{#if compact}
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
  {#if tab === 'watchlist'}{#each additions as addition (addition.key)}<div
        class="watchlist-pending"
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
      </div>{/each}
  {/if}{/if}
<div
  class="youtube-media"
  use:scaleYoutubeMediaScope={{
    columns: compact ? 2 : $appearance.card_columns,
    sidebarOpen: dockOpen && !compact,
  }}
>
  <div
    class="youtube-feed"
    data-feed-scroll
    data-sidebar-resize="xy"
    data-sidebar-resize-origin
  >
    {#each groups as group, index (index)}<FeedGroup title={group.title}>
        {#each group.items as video (video.id)}<YouTubeCard
            {video}
            {busy}
            download={downloadStates[video.id]}
            play={() => play({ id: `youtube:${video.id}`, title: video.title })}
            prepare={() => void prepare(video)}
            change={(key) => void change(video, key)}
          />{:else}<p class="muted">
            {feed ? 'No videos match this view.' : 'Loading your videos…'}
          </p>{/each}
      </FeedGroup>{/each}
    {#if !feed?.items.length}<p class="muted">
        {feed ? 'No videos match this view.' : 'Loading your videos…'}
      </p>{/if}
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
  </div>
  <aside
    class="watchlist-frame"
    data-youtube-watchlist-frame
    aria-label="Watchlist"
    inert={!dockOpen || compact}
    aria-hidden={!dockOpen || compact}
  >
    <div class="watchlist-scale" data-youtube-watchlist-scale>
      <div class="watchlist-header" data-sidebar-resize="xy">
        <Bookmark size={16} />
        <h2>Watchlist</h2>
        <span>{watchlist?.total ?? 0}</span>
      </div>
      <div class="watchlist-add" data-sidebar-resize="xy">
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
      </div>
      <div class="watchlist-items">
        {#each additions as addition (addition.key)}<div
            class="watchlist-pending"
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
          </div>{/each}

        {#each watchlist?.items ?? [] as video (video.id)}<article
            class="watchlist-item"
            data-sidebar-resize="xy"
            data-layout-key={'watchlist:' + video.id}
          >
            <button
              class="watchlist-play"
              aria-label={`Play saved ${video.title}`}
              onclick={() =>
                play({ id: `youtube:${video.id}`, title: video.title })}
              ><div class="watchlist-thumb">
                {#if video.artwork_url}<img
                    src={serverUrl() + video.artwork_url}
                    alt=""
                    loading="lazy"
                  />{:else}<Play size={20} />{/if}{#if video.duration}<span
                    >{duration(video.duration)}</span
                  >{/if}
              </div>
              <div>
                <small>{video.channel || 'Video details pending'}</small>
                <h3>{video.title}</h3>
                {#if video.pending}<small>Waiting for video details…</small
                  >{/if}
              </div></button
            >
            <button
              class="icon-button watchlist-remove"
              aria-label={`Remove saved ${video.title}`}
              title="Remove from watchlist"
              disabled={busy}
              onclick={() => void change(video, 'watchlist')}
              ><X size={14} /></button
            >
          </article>{:else}{#if !additions.length}<p
              class="muted watchlist-empty"
            >
              Save a video from the feed or paste its URL above.
            </p>{/if}{/each}
        {#if watchlist && watchlist.total > 50}<button
            class="secondary"
            onclick={() => {
              tab = 'watchlist';
              offset = 0;
            }}>Browse all {watchlist.total} saved videos</button
          >{/if}
      </div>
    </div>
  </aside>
</div>

<style>
  .actions,
  .filters {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .filters {
    margin: 16px 0;
  }
  .filters label {
    margin: 0;
  }
  .filters label:first-child {
    flex: 1;
    max-width: 330px;
  }
  .filters label:has(input[type='checkbox']) {
    flex-direction: row;
    align-items: center;
  }
  .actions button {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dock-toggle {
    margin-left: auto;
  }
  .youtube-media {
    display: flex;
    gap: 0;
    align-items: flex-start;
    min-width: 0;
    position: relative;
  }
  .youtube-feed {
    flex: 1;
    min-width: 0;
  }
  .watchlist-frame {
    flex-shrink: 0;
    overflow: clip;
    position: sticky;
    top: 0;
    align-self: flex-start;
  }
  .watchlist-scale {
    border-left: 1px solid var(--line);
    background: var(--surface);
    min-height: 500px;
    max-height: calc(100dvh / var(--youtube-media-scale, 1) - 110px);
    display: flex;
    flex-direction: column;
  }
  .watchlist-header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 20px;
    border-bottom: 1px solid var(--line);
    color: var(--accent);
  }
  .watchlist-header h2 {
    margin: 0;
    flex: 1;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.1em;
  }
  .watchlist-header span {
    color: var(--muted);
    font-size: 11px;
  }
  .watchlist-add {
    padding: 0 16px;
    border-bottom: 1px solid var(--line);
  }
  .watchlist-add :global(.inline-form) {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }
  .watchlist-items {
    min-height: 0;
    overflow: auto;
  }
  .watchlist-item {
    position: relative;
    border-bottom: 1px solid var(--line);
    padding: 16px;
  }
  .watchlist-play {
    display: flex;
    width: 100%;
    align-items: flex-start;
    gap: 12px;
    text-align: left;
    background: transparent;
    color: var(--foreground);
    padding: 0;
  }
  .watchlist-play > div:last-child {
    min-width: 0;
    flex: 1;
  }
  .watchlist-play h3 {
    font-size: 13px;
    line-height: 18px;
    margin-top: 7px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .watchlist-play small {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .watchlist-thumb {
    width: 120px;
    aspect-ratio: 16/9;
    flex-shrink: 0;
    position: relative;
    background: #080b10;
    display: grid;
    place-items: center;
    overflow: hidden;
  }
  .watchlist-thumb img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .watchlist-thumb span {
    position: absolute;
    bottom: 3px;
    right: 3px;
    background: #000c;
    color: white;
    padding: 2px 4px;
    font:
      9px ui-monospace,
      monospace;
  }
  .watchlist-remove {
    position: absolute;
    right: 3px;
    top: 3px;
    opacity: 0;
    background: var(--surface-strong);
  }
  .watchlist-item:hover .watchlist-remove,
  .watchlist-item:focus-within .watchlist-remove {
    opacity: 1;
  }
  .watchlist-pending {
    margin: 12px;
    padding: 14px;
    border: 1px solid var(--line);
    font-size: 11px;
    overflow-wrap: anywhere;
  }
  .watchlist-pending p {
    margin: 8px 0 0;
  }
  .watchlist-empty {
    font-size: 12px;
    padding: 24px;
  }
</style>
