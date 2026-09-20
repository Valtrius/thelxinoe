<script lang="ts">
  import { Bookmark, Pin, Check, Download, Play } from '@lucide/svelte';
  import { appearance } from '../appearance';
  import { serverUrl } from '../api';
  import { type Video, duration } from '../youtube-types';
  let {
    video,
    busy,
    download = '',
    play,
    prepare,
    change,
  } = $props<{
    video: Video;
    busy: boolean;
    download?: string;
    play: () => void;
    prepare: () => void;
    change: (key: 'watchlist' | 'pinned' | 'watched') => void;
  }>();
  let imageFailed = $state(false);
</script>

<article
  class="media-tile youtube-card"
  class:watched-fade={$appearance.fade_watched && video.watched}
  data-layout-key={video.id}
  data-sidebar-resize="xy"
>
  <div class="card-actions">
    <button
      aria-label={`Download ${video.title}`}
      title="Download for later"
      disabled={['queued', 'downloading', 'ready'].includes(download)}
      onclick={prepare}><Download size={15} /></button
    >
    <button
      aria-label={`${video.watchlist ? 'Remove' : 'Save'} ${video.title} ${video.watchlist ? 'from' : 'to'} watchlist`}
      title={video.watchlist ? 'Remove from watchlist' : 'Save to watchlist'}
      aria-pressed={video.watchlist}
      disabled={busy}
      onclick={() => change('watchlist')}><Bookmark size={15} /></button
    >
    <button
      aria-label={`Pin ${video.title}`}
      title={video.pinned ? 'Unpin' : 'Keep downloaded video'}
      aria-pressed={video.pinned}
      disabled={busy}
      onclick={() => change('pinned')}><Pin size={15} /></button
    >
    <button
      aria-label={`Watched ${video.title}`}
      title={video.watched ? 'Mark unwatched' : 'Mark watched'}
      aria-pressed={video.watched}
      disabled={busy}
      onclick={() => change('watched')}><Check size={15} /></button
    >
  </div>
  <button
    class="card-primary"
    aria-label={`Play ${video.title}`}
    onclick={play}
  >
    <div class="tile-art">
      {#if video.artwork_url && !imageFailed}<img
          src={`${serverUrl()}${video.artwork_url}`}
          alt=""
          loading="lazy"
          style:object-fit={$appearance.thumbnail_fit}
          onerror={() => (imageFailed = true)}
        />{:else}<span class="thumbnail-placeholder"
          >{video.id.slice(0, 4).toUpperCase()}</span
        ><Play size={30} />{/if}
      {#if video.broadcast !== 'none'}<span class="card-badge live"
          >{video.broadcast}</span
        >{/if}
      {#if video.duration}<span class="card-badge"
          >{duration(video.duration)}</span
        >{/if}
      {#if video.position > 0}<span class="card-badge position"
          >{duration(video.position)}</span
        >{/if}
      {#if video.duration && video.position > 0}<span
          class="card-progress"
          style:width={`${Math.min(100, (video.position / video.duration) * 100)}%`}
        ></span>{/if}
    </div>
    <div class="tile-body card-byline">
      <span class="card-avatar">{(video.channel || '?')[0].toUpperCase()}</span>
      <div>
        <div class="card-overline">
          <span>{video.channel || 'Details pending'}</span
          >{#if video.published_at}<time
              datetime={new Date(video.published_at * 1000).toISOString()}
              >{new Date(video.published_at * 1000).toLocaleDateString(
                undefined,
                { month: 'short', day: 'numeric' },
              )}</time
            >{/if}
        </div>
        <h3>{video.title}</h3>
        {#if video.pending}<p>
            Waiting for video details…
          </p>{:else if !video.available}<p>
            This video is unavailable to your connected account.
          </p>{/if}
        {#if download === 'queued' || download === 'downloading'}<p>
            Downloading for playback…
          </p>{:else if download === 'ready'}<p class="downloaded">
            <Download size={11} /> Downloaded
          </p>{:else if download === 'extractor_authentication_required'}<p>
            Public playback unavailable: sign-in required.
          </p>{:else if ['failed', 'unavailable'].includes(download)}<p>
            Download failed. You can retry.
          </p>{/if}
        {#if video.is_short}<span class="badge">Short</span>{/if}
      </div>
    </div>
  </button>
</article>

<style>
  .youtube-card h3 {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    min-height: 40px;
  }
  .position {
    left: 8px;
    right: auto;
    color: var(--accent);
  }
  .thumbnail-placeholder {
    position: absolute;
    left: 16px;
    bottom: 16px;
    font:
      36px ui-monospace,
      monospace;
    opacity: 0.15;
    color: white;
  }
  .downloaded {
    display: flex;
    align-items: center;
    gap: 5px;
  }
</style>
