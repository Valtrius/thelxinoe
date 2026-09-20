<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from './api';
  import MediaGrid from './ui/MediaGrid.svelte';
  import LibraryCard from './ui/LibraryCard.svelte';
  import {
    clientId,
    restoreQueue,
    type Card,
    type SavedQueue,
  } from './media-state';
  import { time, type MediaChoice } from './playback';
  let { revision, open, play } = $props<{
    revision: number;
    open: (item: Card) => void;
    play: (item: MediaChoice) => void;
  }>();
  type Home = {
    favorites: Card[];
    watch_later: Card[];
    continue_watching: Card[];
    next_up: Card[];
  };
  let home = $state<Home | null>(null),
    queue = $state<SavedQueue | null>(null),
    error = $state('');
  let loading = false;
  $effect(() => {
    if (revision >= 0)
      return untrack(() => {
        const timer = setTimeout(() => void load(), 200);
        return () => clearTimeout(timer);
      });
  });
  async function load() {
    if (loading) return;
    loading = true;
    try {
      const result = await Promise.all([
        api<Home>('/me/home'),
        api<SavedQueue>(`/me/queue/${clientId()}`),
      ]);
      [home, queue] = result;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }
  const shelves = [
    ['continue_watching', 'Continue Watching'],
    ['next_up', 'Next Up'],
    ['watch_later', 'Watch Later'],
    ['favorites', 'Favorites'],
  ] as const;
</script>

{#if error}<p class="error" role="alert">{error}</p>{/if}
{#if home}{#each shelves.filter(([key]) => home?.[key].length) as [key, title] (key)}<section
      class="home-shelf"
      aria-label={title}
    >
      <h2>{title}</h2>
      {#if home[key].length}<MediaGrid
          revision={home[key].map((i) => i.id).join(',')}
          label={title}
          >{#each home[key] as item (item.id)}<LibraryCard
              {item}
              keyPrefix={key}
              open={() => open(item)}
              play={['movie', 'episode', 'track'].includes(item.kind)
                ? () => play(item)
                : undefined}
            />{/each}</MediaGrid
        >{:else}<p class="muted">
          {key === 'continue_watching'
            ? 'Your unfinished movies and episodes will appear here.'
            : key === 'next_up'
              ? 'Continue a series or add it to your favorites to see its next episode.'
              : 'Items you save will appear here.'}
        </p>{/if}
    </section>{:else}<section class="empty home-empty">
      <h2>Pick up where you left off</h2>
      <p>
        Continue watching, next episodes, favorites and saved titles will appear
        here as you use your library.
      </p>
    </section>{/each}{/if}
{#if queue?.items.length}<section class="panel" aria-label="Saved music queue">
    <div class="section-heading">
      <h2>This device’s music queue</h2>
      <button
        class="primary"
        onclick={() => {
          const choice = restoreQueue(queue!);
          if (choice) play(choice);
        }}>{queue.completed ? 'Replay queue' : 'Resume queue'}</button
      >
    </div>
    <p class="muted">
      Track {queue.current_index + 1} of {queue.items.length} · {time(
        queue.position,
      )}
    </p>
    {#each queue.items as id, index (index)}<div class="row">
        <span
          >{index + 1}. {queue.tracks?.find((t) => t.id === id)?.title ??
            'Unavailable track'}</span
        >{#if index === queue.current_index}<span class="badge">Current</span
          >{/if}
      </div>{/each}
  </section>{/if}
