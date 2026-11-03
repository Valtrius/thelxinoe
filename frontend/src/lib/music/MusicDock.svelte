<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { Popover } from 'bits-ui';
  import {
    ChevronDown,
    ListMusic,
    LoaderCircle,
    Pause,
    Play,
    Repeat,
    Repeat1,
    Shuffle,
    SkipBack,
    SkipForward,
    Volume2,
    VolumeX,
    X,
  } from '@lucide/svelte';
  import SeekRange from '../SeekRange.svelte';
  import Button from '../ui/Button.svelte';
  import { appearance, updateAppearance } from '../appearance';
  import { compactViewport } from '../responsive';
  import { choiceIndex } from '../media-state';
  import { time, type MediaChoice } from '../playback';
  import {
    GaplessQueue,
    type MusicState,
    type Repeat as RepeatMode,
  } from '../gapless';
  import MusicArt from './MusicArt.svelte';
  import FavoriteToggle from './FavoriteToggle.svelte';
  import PlayingBars from './PlayingBars.svelte';
  import { musicPlayer } from './player.svelte';

  let { choice, closed } = $props<{
    choice: MediaChoice;
    closed: () => void;
  }>();
  let musicState = $state<MusicState | null>(null),
    error = $state(''),
    busy = $state(true),
    waitingTools = $state(false),
    muted = $state(false),
    queueOpen = $state(false),
    sheetOpen = $state(false);
  let queue = $state<GaplessQueue>();
  let generation = 0;
  let disposed = false;
  let closing = Promise.resolve();
  const compact = $derived(compactViewport.current);
  const volume = $derived(muted ? 0 : $appearance.audio_volume);
  $effect(() => {
    queue?.setVolume(volume);
  });
  $effect(() => {
    const selected = choice;
    untrack(() => void open(selected));
  });
  function release() {
    const previous = queue;
    queue = undefined;
    closing = closing.then(() => previous?.close());
  }
  async function open(selected: MediaChoice) {
    const revision = ++generation;
    release();
    musicState = null;
    busy = true;
    waitingTools = false;
    error = '';
    await closing;
    if (disposed || generation !== revision) return;
    const choices = selected.queue ?? [selected];
    const index = choiceIndex(choices, selected);
    // The chosen track keeps the file and position it was chosen with.
    const list = choices.map((entry: MediaChoice, i: number) =>
      i === index
        ? { ...entry, ...selected, queue: undefined, shuffle: undefined }
        : { ...entry, queue: undefined },
    );
    const owned = new GaplessQueue(
      list,
      (value) => {
        if (generation === revision) musicState = value;
      },
      (e) => {
        if (generation === revision)
          error = e instanceof Error ? e.message : String(e);
      },
      (value) => {
        if (generation === revision) waitingTools = value;
      },
      selected.shuffle ? Math.floor(Math.random() * list.length) : index,
    );
    queue = owned;
    if (selected.shuffle) owned.setShuffle(true);
    try {
      owned.setVolume(volume);
      await owned.start();
    } catch (e) {
      await owned.close();
      if (disposed || generation !== revision) return;
      error = e instanceof Error ? e.message : String(e);
    } finally {
      if (generation === revision) busy = false;
    }
  }
  // A track chosen while the dock leaves brings it back through `open`.
  function stop() {
    generation++;
    release();
    sheet?.close();
    musicPlayer.state = null;
    musicPlayer.current = null;
    musicPlayer.controls = null;
  }
  musicPlayer.stop = stop;
  onDestroy(() => {
    disposed = true;
    stop();
    musicPlayer.stop = null;
  });
  // A finished queue still shows its last track.
  const current = $derived<MediaChoice>(
    (musicState &&
      musicState.queue[
        Math.min(musicState.index, musicState.queue.length - 1)
      ]) ??
      choice,
  );
  const loading = $derived(busy || !!musicState?.loading);
  const finished = $derived(!!musicState?.finished);
  $effect(() => {
    musicPlayer.state = musicState;
    musicPlayer.current = finished ? null : current;
  });
  $effect(() => {
    const owned = queue;
    musicPlayer.controls = owned
      ? { insert: (items, next) => owned.insert(items, next) }
      : null;
  });
  const position = $derived(musicState?.position ?? 0);
  const duration = $derived(musicState?.duration || current.duration || 0);
  const subtitle = $derived(
    [current.artist, current.album].filter(Boolean).join(' · '),
  );
  const status = $derived(
    error
      ? ''
      : waitingTools
        ? 'Waiting for playback tools…'
        : loading
          ? 'Preparing music…'
          : finished
            ? 'Queue finished'
            : '',
  );
  const paused = $derived(musicState?.paused ?? true);
  const repeatLabels: Record<RepeatMode, string> = {
    off: 'Repeat off',
    all: 'Repeat queue',
    one: 'Repeat track',
  };
  const nextRepeat: Record<RepeatMode, RepeatMode> = {
    off: 'all',
    all: 'one',
    one: 'off',
  };
  const repeat = $derived(musicState?.repeat ?? 'off');
  const upcoming = $derived(
    musicState
      ? musicState.queue
          .map((entry, at) => ({ entry, at }))
          .slice(musicState.index + 1)
      : [],
  );
  function changeVolume(value: number) {
    muted = false;
    queue?.setVolume(value);
    updateAppearance({ audio_volume: value });
  }
  let sheet = $state<HTMLDialogElement>();
  $effect(() => {
    if (!sheet) return;
    if (sheetOpen && compact && !sheet.open) sheet.showModal();
    if ((!sheetOpen || !compact) && sheet.open) sheet.close();
  });
  const fraction = (value: number) =>
    duration > 0 ? Math.min(1, Math.max(0, value / duration)) : 0;
</script>

{#snippet seek(className: string)}
  <SeekRange
    class={`player-range w-full focus-visible:outline-2 focus-visible:outline-offset-2 ${className}`}
    style={`--range-fill:${fraction(position)}`}
    aria-label="Music position"
    aria-valuetext={`${time(position)} of ${time(duration)}`}
    min="0"
    max={duration || 1}
    step="0.1"
    {position}
    disabled={!musicState || loading || finished}
    onseek={(to) => void queue?.seek(to)}
  />
{/snippet}
{#snippet toggle(size: 'default' | 'large')}
  <Button
    size="icon"
    variant="player"
    class={`shrink-0 bg-accent-soft text-accent shadow-[inset_0_0_0_1px_var(--line-strong)] ${size === 'large' ? 'size-13' : ''}`}
    aria-label={paused ? 'Play' : 'Pause'}
    title={paused ? 'Play' : 'Pause'}
    disabled={!musicState || finished}
    onclick={() => void queue?.toggle()}
  >
    {#if loading && !error}<LoaderCircle
        size={size === 'large' ? 26 : 20}
        class="animate-spin motion-reduce:animate-none"
      />{:else if paused}<Play
        size={size === 'large' ? 26 : 20}
        fill="currentColor"
      />{:else}<Pause
        size={size === 'large' ? 26 : 20}
        fill="currentColor"
      />{/if}
  </Button>
{/snippet}
{#snippet transport(size: 'default' | 'large')}
  <div
    class={[
      'flex items-center',
      size === 'large' ? 'justify-between' : 'justify-center gap-1',
    ]}
  >
    <Button
      size="icon"
      variant="player"
      class={musicState?.shuffle ? 'text-accent' : 'text-muted'}
      aria-label="Shuffle"
      aria-pressed={musicState?.shuffle ?? false}
      title={musicState?.shuffle ? 'Shuffle on' : 'Shuffle off'}
      disabled={!musicState}
      onclick={() => queue?.setShuffle(!musicState?.shuffle)}
      ><Shuffle size={17} /></Button
    >
    <Button
      size="icon"
      variant="player"
      aria-label="Previous track"
      title="Previous track"
      disabled={!musicState}
      onclick={() => void queue?.previous()}
      ><SkipBack size={20} fill="currentColor" /></Button
    >
    {@render toggle(size)}
    <Button
      size="icon"
      variant="player"
      aria-label="Next track"
      title="Next track"
      disabled={!musicState || finished}
      onclick={() => void queue?.skip()}
      ><SkipForward size={20} fill="currentColor" /></Button
    >
    <Button
      size="icon"
      variant="player"
      class={repeat === 'off' ? 'text-muted' : 'text-accent'}
      aria-label={repeatLabels[repeat]}
      title={repeatLabels[repeat]}
      disabled={!musicState}
      onclick={() => queue?.setRepeat(nextRepeat[repeat])}
      >{#if repeat === 'one'}<Repeat1 size={17} />{:else}<Repeat
          size={17}
        />{/if}</Button
    >
  </div>
{/snippet}
{#snippet volumeControl(className: string)}
  <div class={`flex items-center gap-1 ${className}`}>
    <Button
      size="icon"
      variant="player"
      aria-label={volume === 0 ? 'Unmute' : 'Mute'}
      title={volume === 0 ? 'Unmute' : 'Mute'}
      onclick={() => (muted = !muted)}
      >{#if volume === 0}<VolumeX size={18} />{:else}<Volume2
          size={18}
        />{/if}</Button
    >
    <input
      class="player-range w-full min-w-10 focus-visible:outline-2 focus-visible:outline-offset-2"
      style:--range-fill={volume}
      aria-label="Music volume"
      type="range"
      min="0"
      max="1"
      step="0.01"
      value={volume}
      oninput={(event) => changeVolume(Number(event.currentTarget.value))}
    />
  </div>
{/snippet}
{#snippet queueList(from: number)}
  <ol class="m-0 list-none p-0">
    {#each (musicState?.queue ?? []).slice(from) as entry, offset (`${from + offset}:${entry.id}`)}
      {@const at = from + offset}
      {@const playing = at === musicState?.index}
      <li
        class={[
          'group/queue flex min-h-10 items-center gap-1 border-t border-line pr-1.5 first:border-t-0',
          playing && 'bg-accent-soft text-accent',
        ]}
      >
        <button
          type="button"
          class="flex min-w-0 flex-1 items-center gap-2.5 bg-transparent py-1.5 pl-3 text-left text-[12.5px] text-inherit"
          aria-label={`Play ${entry.title}`}
          aria-current={playing ? 'true' : undefined}
          onclick={() => void queue?.jump(at)}
        >
          <span class="flex w-5 shrink-0 justify-end text-[11px] text-muted"
            >{#if playing}<PlayingBars {paused} />{:else}{at + 1}{/if}</span
          >
          <span class="min-w-0 flex-1 truncate"
            >{entry.title}{#if entry.artist}<span
                class="block truncate text-[11px] text-muted"
                >{entry.artist}</span
              >{/if}</span
          >
          {#if entry.duration}<span
              class="shrink-0 text-[11px] text-muted tabular-nums"
              >{time(entry.duration)}</span
            >{/if}
        </button>
        {#if !playing}<Button
            variant="player"
            size="compact-icon"
            class="text-muted opacity-0 group-hover/queue:opacity-100 focus-visible:opacity-100 compact:opacity-100"
            aria-label={`Remove ${entry.title} from the queue`}
            title="Remove from the queue"
            onclick={() => void queue?.remove(at)}><X size={14} /></Button
          >{/if}
      </li>
    {/each}
  </ol>
{/snippet}

<section
  class="music-dock theme-dark relative z-40 min-h-0 border-t border-line-strong bg-background text-foreground shadow-[0_-8px_24px_rgb(0_0_0/0.16)] [text-shadow:0_0_2px_var(--background),0_0_6px_var(--background)]"
  aria-label="Music player"
>
  {#if compact}<!-- Phones seek in Now playing; the bar shows how far the track is. -->
    <div
      class="absolute inset-x-0 top-0 h-0.5 -translate-y-full bg-foreground/20"
      aria-hidden="true"
    >
      <div
        class="h-full bg-accent"
        style:width={`${fraction(position) * 100}%`}
      ></div>
    </div>{:else}<div class="absolute inset-x-0 top-0 -translate-y-1/2">
      {@render seek('')}
    </div>{/if}
  {#if compact}
    <div class="flex items-center gap-2 py-2 pr-2 pl-3">
      <button
        type="button"
        class="flex min-w-0 flex-1 items-center gap-3 bg-transparent p-0 text-left text-inherit"
        aria-label="Open now playing"
        onclick={() => (sheetOpen = true)}
      >
        <MusicArt
          url={current.artwork_url}
          class="size-10 border border-white/15"
          sizes="3rem"
        />
        <span class="min-w-0">
          <b class="block truncate text-[13px] font-semibold">{current.title}</b
          >
          <span
            class={[
              'block truncate text-[11px]',
              error ? 'text-danger' : 'text-muted',
            ]}
            role={error ? 'alert' : 'status'}
            >{error || status || subtitle}</span
          >
        </span>
      </button>
      {@render toggle('default')}
      <Button
        size="icon"
        variant="player"
        aria-label="Next track"
        title="Next track"
        disabled={!musicState || finished}
        onclick={() => void queue?.skip()}
        ><SkipForward size={20} fill="currentColor" /></Button
      >
    </div>
  {:else}
    <div
      class="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-4 px-4 pt-3 pb-2.5 narrow:grid-cols-[minmax(0,1fr)_auto] narrow:gap-3"
    >
      <div class="flex min-w-0 items-center gap-3">
        <MusicArt
          url={current.artwork_url}
          class="size-12 border border-white/15"
          sizes="3rem"
        />
        <div class="min-w-0">
          <b class="block truncate text-[13px] font-semibold">{current.title}</b
          >
          <span
            class={[
              'flex min-w-0 items-center gap-1.5 truncate text-[11px]',
              error ? 'text-danger' : 'text-muted',
            ]}
            role={error ? 'alert' : 'status'}
            >{#if status && !finished}<LoaderCircle
                size={12}
                class="shrink-0 animate-spin motion-reduce:animate-none"
              />{/if}<span class="truncate">{error || status || subtitle}</span
            ></span
          >
        </div>
        {#if !finished && current.id}<FavoriteToggle
            id={current.id}
            variant="player"
            class="shrink-0 text-muted"
          />{/if}
      </div>
      <div class="grid justify-items-center gap-0.5">
        {@render transport('default')}
        <span class="text-[11px] text-muted tabular-nums" aria-hidden="true"
          >{time(position)} / {time(duration)}</span
        >
      </div>
      <div class="flex items-center justify-end gap-2 narrow:col-span-2">
        {#if musicState && musicState.count > 1}<span
            class="text-[11px] whitespace-nowrap text-muted tabular-nums"
            >{Math.min(musicState.index + 1, musicState.count)} of {musicState.count}</span
          >{/if}
        <Popover.Root bind:open={queueOpen}>
          <Popover.Trigger>
            {#snippet child({ props })}<Button
                {...props}
                size="icon"
                variant="player"
                class={queueOpen ? 'text-accent' : ''}
                aria-label="Queue"
                title="Queue"
                disabled={!musicState}><ListMusic size={18} /></Button
              >{/snippet}
          </Popover.Trigger>
          <Popover.Portal>
            <Popover.Content
              side="top"
              align="end"
              sideOffset={10}
              aria-label="Queue"
              class="theme-dark z-50 grid max-h-[min(30rem,70vh)] w-80 grid-rows-[auto_minmax(0,1fr)] border border-line-strong bg-surface-strong text-foreground shadow-2xl"
            >
              <div
                class="flex items-center justify-between gap-2 border-b border-line py-1.5 pr-1.5 pl-3"
              >
                <span
                  class="text-[10px] font-[650] tracking-[0.17em] text-muted uppercase"
                  >Queue{musicState
                    ? ` · ${Math.min(musicState.index + 1, musicState.count)} of ${musicState.count}`
                    : ''}</span
                >
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={!upcoming.length}
                  onclick={() => queue?.clear()}>Clear</Button
                >
              </div>
              <div class="min-h-0 overflow-y-auto">
                {@render queueList(0)}
              </div>
            </Popover.Content>
          </Popover.Portal>
        </Popover.Root>
        {@render volumeControl('w-28')}
        <Button
          size="icon"
          variant="player"
          class="text-muted"
          aria-label="Close player"
          title="Close player"
          onclick={closed}><X size={18} /></Button
        >
      </div>
    </div>
  {/if}
</section>
{#if compact}<dialog
    bind:this={sheet}
    class="theme-dark m-0 h-dvh max-h-none w-full max-w-none overflow-y-auto border-0 bg-background p-0 text-foreground backdrop:bg-black/60 desktop-shell:mt-8 desktop-shell:h-[calc(100dvh-32px)]"
    aria-label="Now playing"
    oncancel={(event) => {
      event.preventDefault();
      sheetOpen = false;
    }}
    onclose={() => (sheetOpen = false)}
  >
    <div
      class="mx-auto grid max-w-md content-start gap-4 px-5 pt-3 pb-8 [text-shadow:0_0_2px_var(--background),0_0_6px_var(--background)]"
    >
      <div class="flex items-center justify-between">
        <Button
          size="icon"
          variant="player"
          aria-label="Close now playing"
          title="Close now playing"
          onclick={() => (sheetOpen = false)}><ChevronDown size={22} /></Button
        >
        <span
          class="text-[10px] font-[650] tracking-[0.17em] text-muted uppercase"
          >Now playing</span
        >
        <Button
          size="icon"
          variant="player"
          class="text-muted"
          aria-label="Close player"
          title="Close player"
          onclick={() => {
            sheetOpen = false;
            closed();
          }}><X size={18} /></Button
        >
      </div>
      <MusicArt
        url={current.artwork_url}
        class="aspect-square w-full border border-white/15 shadow-2xl"
        sizes="28rem"
      />
      <div class="flex items-start gap-3">
        <div class="min-w-0 flex-1">
          <h2 class="mb-0 truncate text-lg">{current.title}</h2>
          <p
            class={[
              'mb-0 truncate text-xs',
              error ? 'text-danger' : 'text-muted',
            ]}
          >
            {error || status || subtitle}
          </p>
        </div>
        {#if !finished && current.id}<FavoriteToggle
            id={current.id}
            variant="player"
            class="shrink-0 text-muted"
          />{/if}
      </div>
      <div class="grid gap-1">
        {@render seek('')}
        <div
          class="flex justify-between text-[11px] text-muted tabular-nums"
          aria-hidden="true"
        >
          <span>{time(position)}</span><span>{time(duration)}</span>
        </div>
      </div>
      {@render transport('large')}
      {@render volumeControl('')}
      {#if upcoming.length}<div>
          <p
            class="mb-1.5 text-[10px] font-[650] tracking-[0.17em] text-muted uppercase"
          >
            Up next · {upcoming.length}
          </p>
          <div class="border border-line bg-surface">
            {@render queueList((musicState?.index ?? 0) + 1)}
          </div>
        </div>{/if}
    </div>
  </dialog>{/if}
