<script lang="ts">
  import SectionHeading from './ui/SectionHeading.svelte';
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { onDestroy, untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { playbackRequest, time, type MediaChoice } from './playback';
  import { api, ApiError } from './api';
  import { toolsApi } from './providers/tools-api';
  import { toolOperationActive } from './providers/tools-presentation';
  import { LoaderCircle } from '@lucide/svelte';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';

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
    busy = $state(true),
    waitingTools = $state(false);
  const preparing = $derived(busy || view?.status === 'starting');
  let unsubscribe: UnlistenFn | undefined,
    generation = 0;
  let preparation = new AbortController();
  $effect(() => {
    const selected = choice;
    untrack(() => void open(selected));
  });
  async function open(selected: MediaChoice) {
    preparation.abort();
    preparation = new AbortController();
    const revision = ++generation;
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
      if (!selected.restore)
        await playbackRequest(async () => {
          const snapshot = await toolsApi.get();
          const mpv = snapshot.tools.find((tool) => tool.id === 'mpv');
          if (
            mpv?.preference.source === 'managed' &&
            (!mpv.selectedPath || mpv.diagnostic?.detected === false) &&
            snapshot.operation.tool === 'mpv' &&
            toolOperationActive(snapshot.operation)
          )
            throw new ApiError(
              503,
              'tools_preparing',
              'Waiting for playback tools…',
            );
        }, pending);
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
      const listener = await listen<View>('mpv-state', (event) => {
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
      if (generation !== revision) {
        listener();
        return;
      }
      unsubscribe = listener;
      const result = selected.restore
        ? await invoke<View>('mpv_state')
        : await invoke<View>('mpv_play', {
            choice: selected,
            queue: selected.queue ?? null,
            music: selected.kind === 'track',
          });
      if (generation === revision) view = result;
    } catch (e) {
      if (generation === revision) error = String(e);
    } finally {
      if (generation === revision) busy = false;
    }
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
      await invoke('mpv_command', { command, value: value ?? null });
    } catch (e) {
      if (generation === revision) error = String(e);
    }
  }
  onDestroy(() => {
    generation++;
    preparation.abort();
    unsubscribe?.();
    if (preparing)
      void invoke('mpv_command', { command: 'stop', value: null }).catch(
        () => {},
      );
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
            >Position<input
              class={formControlClass}
              aria-label="Native playback position"
              type="range"
              min="0"
              max={view.duration}
              step="0.1"
              value={view.position}
              onchange={(e) =>
                void command('seek', Number(e.currentTarget.value))}
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
