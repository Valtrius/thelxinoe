<script lang="ts">
  import SectionHeading from './ui/SectionHeading.svelte';
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import SeekRange from './SeekRange.svelte';
  import { formControlClass } from './ui/styles';
  import { onDestroy, untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { playbackRequest, time, type MediaChoice } from './playback';
  import { api } from './api';
  import { waitForTools, type NativeView as View } from './native-playback';
  import { LoaderCircle } from '@lucide/svelte';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';

  let { choice, closed } = $props<{
    choice: MediaChoice;
    closed: (choice: MediaChoice) => void;
  }>();
  let view = $state<View | null>(null),
    error = $state(''),
    busy = $state(true),
    waitingTools = $state(false);
  const preparing = $derived(busy || view?.status === 'starting');
  let unsubscribe: UnlistenFn | undefined,
    generation = 0,
    run = '';
  let preparation = new AbortController();
  $effect(() => {
    const selected = choice;
    untrack(() => void open(selected));
  });
  async function open(selected: MediaChoice) {
    preparation.abort();
    preparation = new AbortController();
    const revision = ++generation;
    const own =
      selected.restore && selected.run ? selected.run : crypto.randomUUID();
    run = own;
    busy = true;
    waitingTools = false;
    error = '';
    view = null;
    const pending = {
      signal: preparation.signal,
      waiting: (value: boolean) => {
        if (generation === revision) waitingTools = value;
      },
    };
    try {
      if (!selected.restore) await waitForTools(pending);
      if (generation !== revision) return;
      if (selected.resolveYoutube)
        await playbackRequest(
          () =>
            api('/online/youtube/resolve', 'POST', {
              video_id: selected.id.slice('youtube:'.length),
            }),
          pending,
        );
      if (generation !== revision) return;
      unsubscribe?.();
      let reported = false;
      const listener = await listen<View>('mpv-state', (event) => {
        if (generation === revision && event.payload.run === own) {
          reported = true;
          view = event.payload;
          if (event.payload.error) error = event.payload.error;
          if (event.payload.status === 'stopped' && !busy) closed(selected);
        }
      });
      if (generation !== revision) {
        listener();
        return;
      }
      unsubscribe = listener;
      const result = selected.restore
        ? await invoke<View>('mpv_state', { run: own })
        : await invoke<View>('mpv_play', {
            choice: selected,
            queue: selected.queue ?? null,
            music: selected.kind === 'track',
            run: own,
          });
      // Closing while MPV was launching could not reach the run yet.
      if (generation !== revision) void stop(own);
      // Its own reports may already be newer than this answer.
      else if (!reported) view = result;
    } catch (e) {
      if (generation === revision) error = String(e);
    } finally {
      if (generation === revision) busy = false;
    }
  }
  function stop(target: string) {
    return invoke('mpv_command', {
      command: 'stop',
      value: null,
      run: target,
    }).catch(() => {});
  }
  async function close() {
    const closing = choice;
    const revision = ++generation;
    preparation.abort();
    await command('stop');
    if (generation === revision) {
      busy = false;
      view = null;
    }
    closed(closing);
  }
  async function command(command: string, value?: number) {
    const revision = generation;
    try {
      await invoke('mpv_command', { command, value: value ?? null, run });
    } catch (e) {
      if (generation === revision) error = String(e);
    }
  }
  onDestroy(() => {
    generation++;
    preparation.abort();
    unsubscribe?.();
    if (preparing) void stop(run);
  });
</script>

{#if choice.kind === 'track' || preparing || error}<Panel
    aria-label="Native player"
  >
    <SectionHeading>
      <h2>{view?.title ?? choice.title}</h2>
      <Button variant="secondary" size="form" onclick={() => void close()}
        >Close player</Button
      >
    </SectionHeading>
    {#if error}<Notice variant="error" role="alert">{error}</Notice
      >{/if}{#if preparing}<p
        role="status"
        class="flex items-center justify-center gap-3 py-10"
      >
        <LoaderCircle
          size={32}
          class="animate-spin text-accent motion-reduce:animate-none"
        />{waitingTools ? 'Waiting for playback tools…' : 'Preparing playback…'}
      </p>{/if}
    {#if error}<Button
        variant="secondary"
        size="form"
        onclick={() => void open(choice)}>Retry playback</Button
      >{/if}
    {#if view}<p class="text-muted">
        {view.music
          ? 'Music plays through MPV.'
          : 'Video plays in its MPV window.'} · {view.status}
      </p>
      <div
        class="flex flex-wrap items-center gap-[15px] [&_input]:w-full [&_input]:p-0 [&_label]:min-w-40 [&_label]:flex-1"
      >
        <Button
          size="form"
          disabled={preparing || view.status === 'stopped'}
          onclick={() => void command('pause')}
          >{view.paused ? 'Play' : 'Pause'}</Button
        >{#if view.music}<Button
            variant="secondary"
            size="form"
            onclick={() => void command('next')}>Next track</Button
          >{/if}<span
          >{view.duration === 0
            ? 'Live'
            : `${time(view.position)} / ${time(view.duration)}`}</span
        >{#if view.duration > 0}<FormField
            >Position<SeekRange
              class={formControlClass}
              aria-label="Native playback position"
              min="0"
              max={view.duration}
              step="0.1"
              position={view.position}
              onseek={(to) => void command('seek', to)}
            /></FormField
          >{/if}<FormField
          >Volume<input
            class={formControlClass}
            aria-label="Native playback volume"
            type="range"
            min="0"
            max="100"
            step="1"
            value="100"
            oninput={(e) =>
              void command('volume', Number(e.currentTarget.value))}
          /></FormField
        >
      </div>
      {#if view.count > 1}<p class="text-muted">
          Track {view.index + 1} of {view.count}
        </p>{/if}{/if}
  </Panel>{/if}
