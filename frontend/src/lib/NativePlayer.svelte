<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { time, type MediaChoice } from './playback';
  type View = {
    file_id: string;
    generation: string;
    media_id: string;
    title: string;
    position: number;
    duration: number;
    paused: boolean;
    status: string;
    music: boolean;
    index: number;
    count: number;
    error: string | null;
    video_ready: boolean;
  };
  let { choice, closed } = $props<{
    choice: MediaChoice;
    closed: (choice: MediaChoice) => void;
  }>();
  let view = $state<View | null>(null),
    error = $state(''),
    busy = $state(false);
  let unsubscribe: UnlistenFn | undefined,
    generation = 0;
  $effect(() => {
    const selected = choice;
    untrack(() => void open(selected));
  });
  async function open(selected: MediaChoice) {
    const revision = ++generation;
    busy = true;
    error = '';
    try {
      unsubscribe?.();
      unsubscribe = await listen<View>('mpv-state', (event) => {
        if (generation === revision) {
          view = event.payload;
          if (event.payload.error) error = event.payload.error;
          if (
            event.payload.status === 'stopped' &&
            event.payload.media_id === selected.id &&
            !busy
          )
            closed(selected);
        }
      });
      const result = selected.restore
        ? await invoke<View>('mpv_state')
        : await invoke<View>('mpv_play', {
            choice: selected,
            queue: selected.queue ?? null,
            music: selected.kind === 'track',
          });
      if (generation === revision) view = result;
    } catch (e) {
      error = String(e);
    } finally {
      if (generation === revision) busy = false;
    }
  }
  async function command(command: string, value?: number) {
    try {
      await invoke('mpv_command', { command, value: value ?? null });
    } catch (e) {
      error = String(e);
    }
  }
  onDestroy(() => {
    generation++;
    unsubscribe?.();
  });
</script>

{#if choice.kind === 'track'}<section class="panel" aria-label="Native player">
    <div class="section-heading">
      <h2>{view?.title ?? choice.title}</h2>
      <button
        class="secondary"
        onclick={async () => {
          const closing = choice;
          await command('stop');
          closed(closing);
        }}>Close player</button
      >
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}{#if busy}<p
        role="status"
      >
        Opening MPV…
      </p>{/if}
    {#if view}<p class="muted">
        {view.music
          ? 'Music plays through MPV.'
          : 'Video plays in its MPV window.'} · {view.status}
      </p>
      <div class="controls">
        <button
          class="primary"
          disabled={busy || view.status === 'stopped'}
          onclick={() => void command('pause')}
          >{view.paused ? 'Play' : 'Pause'}</button
        >{#if view.music}<button
            class="secondary"
            onclick={() => void command('next')}>Next track</button
          >{/if}<span
          >{view.duration === 0
            ? 'Live'
            : `${time(view.position)} / ${time(view.duration)}`}</span
        >{#if view.duration > 0}<label
            >Position<input
              aria-label="Native playback position"
              type="range"
              min="0"
              max={view.duration}
              step="0.1"
              value={view.position}
              onchange={(e) =>
                void command('seek', Number(e.currentTarget.value))}
            /></label
          >{/if}<label
          >Volume<input
            aria-label="Native playback volume"
            type="range"
            min="0"
            max="100"
            step="1"
            value="100"
            oninput={(e) =>
              void command('volume', Number(e.currentTarget.value))}
          /></label
        >
      </div>
      {#if view.count > 1}<p class="muted">
          Track {view.index + 1} of {view.count}
        </p>{/if}{/if}
  </section>{:else if error}<p class="error" role="alert">{error}</p>{/if}

<style>
  .controls {
    display: flex;
    align-items: center;
    gap: 15px;
    flex-wrap: wrap;
  }
  .controls label {
    flex: 1;
    min-width: 160px;
  }
  .controls input {
    padding: 0;
    width: 100%;
  }
</style>
