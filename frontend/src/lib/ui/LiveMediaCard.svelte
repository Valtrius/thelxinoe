<script lang="ts">
  import { Play, Users, Radio, X } from '@lucide/svelte';
  import { appearance } from '../appearance';
  let {
    id,
    platform,
    name,
    title,
    category,
    viewers,
    thumbnail,
    avatar,
    startedAt,
    live = true,
    play,
    remove,
    error,
  } = $props<{
    id: string;
    platform: 'twitch' | 'kick';
    name: string;
    title: string;
    category: string;
    viewers: number;
    thumbnail?: string | null;
    avatar?: string | null;
    startedAt?: string | null;
    live?: boolean | null;
    play: () => void;
    remove?: () => void;
    error?: string | null;
  }>();
  let imageFailed = $state(false);
  const elapsed = $derived(
    startedAt
      ? Math.max(0, Math.floor((Date.now() - Date.parse(startedAt)) / 60000))
      : null,
  );
</script>

<article
  class="media-tile live-tile"
  data-layout-key={id}
  data-sidebar-resize="xy"
  data-platform={platform}
>
  <div class="card-actions">
    <button title={`Play ${name}`} aria-label={`Play ${name}`} onclick={play}
      ><Play size={15} /></button
    >{#if remove}<button
        title="Stop tracking"
        aria-label={`Stop tracking ${name}`}
        onclick={remove}><X size={15} /></button
      >{/if}
  </div>
  <button class="card-primary" aria-label={`Watch ${name}`} onclick={play}>
    <div class="tile-art">
      {#if thumbnail && !imageFailed}<img
          src={thumbnail}
          referrerpolicy="no-referrer"
          alt=""
          loading="lazy"
          style:object-fit={$appearance.thumbnail_fit}
          onerror={() => (imageFailed = true)}
        />{:else}<Radio size={40} />{/if}
      <span class="card-badge live" class:offline={!live}
        >{live === null ? 'Unknown' : live ? 'Live' : 'Offline'}{#if live}
          · <Users size={10} />{viewers.toLocaleString()}{/if}</span
      >
      {#if elapsed !== null && Number.isFinite(elapsed) && live}<span
          class="card-badge"
          >{Math.floor(elapsed / 60)}:{String(elapsed % 60).padStart(
            2,
            '0',
          )}</span
        >{/if}
    </div>
    <div class="tile-body">
      <div class="card-byline">
        {#if avatar}<img class="card-avatar" src={avatar} alt="" />{:else}<span
            class="card-avatar">{name[0]?.toUpperCase()}</span
          >{/if}
        <div>
          <h3>{name}</h3>
          <p class="stream-title">
            {title || (live ? 'Live stream' : 'Not live right now')}
          </p>
        </div>
      </div>
      <p class="stream-category">{category || platform}</p>
      {#if error}<p role="status">{error}</p>{/if}
    </div>
  </button>
</article>

<style>
  .card-badge.live {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .stream-title {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    min-height: 33px;
  }
  .stream-category {
    padding-left: 48px;
  }
  [data-platform='kick'] .card-badge.live {
    color: #72ff43;
  }
</style>
