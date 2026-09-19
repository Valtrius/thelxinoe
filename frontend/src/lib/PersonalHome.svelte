<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from './api';
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
{#if home}{#each shelves as [key, title] (key)}<section
      class="panel"
      aria-label={title}
    >
      <h2>{title}</h2>
      {#each home[key] as item (item.id)}<div class="row">
          <button class="secondary" onclick={() => open(item)}
            >{item.show_title
              ? `${item.show_title} · `
              : ''}{item.title}</button
          >
          {#if item.duration}<small
              >{time(item.position ?? 0)} / {time(item.duration)}</small
            >{/if}
          {#if ['movie', 'episode', 'track'].includes(item.kind)}<button
              class="primary"
              disabled={!item.available}
              onclick={() => play(item)}
              >{key === 'continue_watching' ? 'Resume' : 'Play'}</button
            >{/if}
        </div>{:else}<p class="muted">
          {key === 'continue_watching'
            ? 'Your unfinished movies and episodes will appear here.'
            : key === 'next_up'
              ? 'Continue a series or add it to your favorites to see its next episode.'
              : 'Items you save will appear here.'}
        </p>{/each}
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
