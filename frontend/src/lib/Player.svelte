<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
  import { PlaybackActivity } from './playback-activity';
  import Hls from 'hls.js';
  import {
    AlertCircle,
    AudioLines,
    Captions,
    Settings2,
    LoaderCircle,
    Maximize,
    Minimize,
    Pause,
    Play,
    Volume2,
    VolumeX,
    X,
  } from '@lucide/svelte';
  import SegmentSkip from './SegmentSkip.svelte';
  import PlayerOption from './PlayerOption.svelte';
  import { playerReveal } from './player-reveal';
  import { api } from './api';
  import { appearance, updateAppearance } from './appearance';
  import {
    capabilities,
    mediaUrl,
    prepare,
    time,
    replayGain,
    type MediaChoice,
    type MediaInfo,
    type Playback,
  } from './playback';
  let {
    choice,
    closed,
    ended,
    resizable = false,
  } = $props<{
    choice: MediaChoice;
    closed: () => void;
    ended?: () => void;
    resizable?: boolean;
  }>();
  const playerId = $props.id();
  let resizedHeight = $state<number | null>(null);
  let viewportHeight = $state(window.innerHeight);
  const minimumHeight = 210;
  const maximumHeight = $derived(Math.max(minimumHeight, viewportHeight - 120));
  const boundedHeight = $derived(
    resizedHeight === null
      ? undefined
      : Math.max(minimumHeight, Math.min(maximumHeight, resizedHeight)),
  );
  let drag: { pointer: number; y: number; height: number } | undefined;
  function resizeBy(height: number, persist = false) {
    const next = Math.max(minimumHeight, Math.min(maximumHeight, height));
    resizedHeight = next;
    if (persist) updateAppearance({ player_height: next });
  }
  function resetHeight() {
    resizedHeight = null;
    updateAppearance({ player_height: null });
  }
  function finishResize() {
    if (!drag) return;
    drag = undefined;
    updateAppearance({ player_height: resizedHeight });
  }
  let player = $state<HTMLVideoElement>() as HTMLVideoElement;
  let container = $state<HTMLElement>() as HTMLElement;
  let active = $state<Playback | null>(null),
    info = $state<MediaInfo | null>(null),
    error = $state(''),
    busy = $state(true),
    buffering = $state(true),
    paused = $state(true),
    position = $state(0),
    quality = $state('auto'),
    audio = $state<number | null>(null),
    subtitle = $state('off'),
    controlsVisible = $state(true),
    controlFocused = $state(false),
    optionMenu = $state<string | null>(null),
    playerHeight = $state(0),
    fullscreen = $state(false),
    muted = $state(false),
    speed = $state(1);
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let audibleVolume = 1;
  let lastSubtitle: string | null = null;
  $effect.pre(() => {
    const savedHeight = $appearance.player_height;
    if (resizable && !drag) resizedHeight = savedHeight;
  });
  const loading = $derived(!error && (busy || buffering));
  const controlsShown = $derived(
    controlsVisible ||
      paused ||
      loading ||
      !!error ||
      controlFocused ||
      !!optionMenu,
  );
  let hls: Hls | undefined,
    sequence = 0,
    generation = 0,
    reporting: Promise<unknown> = Promise.resolve();
  let audioContext: AudioContext | undefined,
    audioElement: HTMLVideoElement | undefined,
    gain: GainNode | undefined;
  const cueTimes = new WeakMap<TextTrackCue, { start: number; end: number }>();
  $effect(() => {
    if ($appearance.audio_volume > 0) audibleVolume = $appearance.audio_volume;
    if (player) {
      player.volume = $appearance.audio_volume;
      player.muted = muted;
    }
  });
  $effect(() => {
    // Restart the idle delay when playback resumes or a blocking overlay closes.
    if (!paused && !loading && !controlFocused && !optionMenu && !error)
      untrack(revealControls);
  });
  function revealControls() {
    controlsVisible = true;
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      if (
        !container?.querySelector(
          '.player-header:hover, .player-controls:hover',
        )
      )
        controlsVisible = false;
    }, 2500);
  }
  function changeVolume(value: number) {
    value = Math.max(0, Math.min(1, value));
    muted = false;
    player.volume = value;
    updateAppearance({ audio_volume: value });
  }
  function toggleMute() {
    if ($appearance.audio_volume === 0) changeVolume(audibleVolume);
    else muted = !muted;
  }
  async function toggleFullscreen() {
    try {
      if (document.fullscreenElement === container)
        await document.exitFullscreen();
      else await container.requestFullscreen();
    } catch {
      // A browser may deny fullscreen without interrupting playback.
    }
    revealControls();
  }
  function setSubtitle(value: string) {
    if (value === 'off') {
      if (subtitle && subtitle !== 'off') lastSubtitle = subtitle;
    } else {
      lastSubtitle = value;
    }
    subtitle = value;
    selectSubtitle();
  }
  function toggleSubtitles() {
    const subtitles = active?.subtitles ?? [];
    if (!subtitles.length) return;
    if (subtitle !== 'off') {
      setSubtitle('off');
      return;
    }
    const selected = [lastSubtitle, active?.selected_subtitle]
      .filter((id): id is string => !!id && id !== 'off')
      .find((id) => subtitles.some((track) => track.id === id));
    setSubtitle(selected ?? subtitles[0].id);
  }
  function changeSpeed(delta: number) {
    speed = Math.max(0.25, Math.min(2, Math.round((speed + delta) * 4) / 4));
    player.playbackRate = speed;
  }
  function parseFrameRate(value?: string) {
    if (!value) return undefined;
    const [numerator, denominator = '1'] = value.split('/');
    const rate = Number(numerator) / Number(denominator);
    return Number.isFinite(rate) && rate > 0 ? rate : undefined;
  }
  function frameRate() {
    const stream = active?.probe.streams?.find(
      (candidate) => candidate.codec_type === 'video',
    );
    return (
      parseFrameRate(stream?.avg_frame_rate) ??
      parseFrameRate(stream?.r_frame_rate) ??
      30
    );
  }
  function stepFrame(direction: -1 | 1) {
    if (!active || !active.video || active.live || busy || !player.paused)
      return;
    const upperBound = Number.isFinite(player.duration)
      ? player.duration
      : Math.max(0, active.duration - active.timeline_start);
    player.currentTime = Math.max(
      0,
      Math.min(upperBound, player.currentTime + direction / frameRate()),
    );
    position = Math.min(
      active.duration,
      player.currentTime + active.timeline_start,
    );
    void report('paused');
  }
  function seekBy(seconds: number) {
    if (!active || active.live || (busy && active.mode !== 'direct')) return;
    void seek(Math.max(0, Math.min(active.duration, position + seconds)));
  }
  function isInteractiveTarget(target: EventTarget | null) {
    return (
      target instanceof Element &&
      !!target.closest(
        'button, input, select, textarea, a, [contenteditable]:not([contenteditable="false"]), [role="menuitem"], [role="menuitemradio"]',
      )
    );
  }
  function handleShortcut(event: KeyboardEvent) {
    if (
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      isInteractiveTarget(event.target)
    )
      return;
    const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
    switch (key) {
      case ' ':
      case 'k':
        void toggle();
        break;
      case 'ArrowLeft':
        seekBy(-5);
        break;
      case 'ArrowRight':
        seekBy(5);
        break;
      case 'j':
        seekBy(-10);
        break;
      case 'l':
        seekBy(10);
        break;
      case 'ArrowUp':
        changeVolume($appearance.audio_volume + 0.05);
        break;
      case 'ArrowDown':
        changeVolume($appearance.audio_volume - 0.05);
        break;
      case 'm':
        toggleMute();
        break;
      case 'f':
        void toggleFullscreen();
        break;
      case 'c':
        toggleSubtitles();
        break;
      case '.':
        stepFrame(1);
        break;
      case ',':
        stepFrame(-1);
        break;
      case '>':
        changeSpeed(0.25);
        break;
      case '<':
        changeSpeed(-0.25);
        break;
      default:
        return;
    }
    event.preventDefault();
    revealControls();
  }
  function ready() {
    if (
      active &&
      !player.seeking &&
      player.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA
    ) {
      buffering = false;
      if (activity.setActive(!player.paused))
        void report(player.paused ? 'paused' : 'playing');
    }
  }
  let activity = new PlaybackActivity();
  function waiting() {
    if (!active) return;
    buffering = true;
    if (activity.setActive(false)) void report('paused');
  }
  const timer = setInterval(() => {
    void report(paused || buffering ? 'paused' : 'playing');
  }, 10000);
  $effect(() => {
    const current = choice;
    untrack(() => void open(current));
  });
  async function open(current: MediaChoice) {
    const revision = ++generation;
    busy = true;
    buffering = true;
    optionMenu = null;
    info = null;
    error = '';
    await stop();
    if (revision !== generation) return;
    position = 0;
    try {
      const loaded = await api<MediaInfo>(`/catalog/${current.id}/playback`);
      if (revision !== generation) return;
      info = loaded;
      quality = loaded.preferences.quality;
      audio = null;
      subtitle = loaded.preferences.subtitles ? '' : 'off';
      await start(undefined, revision);
    } catch (e) {
      if (revision === generation) error = String(e);
    } finally {
      if (revision === generation) busy = false;
    }
  }
  async function start(at?: number, revision = generation) {
    const result = await prepare(
      choice,
      {
        quality,
        audio,
        subtitle: subtitle || null,
        capabilities: capabilities(),
      },
      at,
    );
    if (revision !== generation) {
      await api(`/playback/${result.id}/progress`, 'POST', {
        sequence: 0,
        position: result.position,
        state: 'stopped',
      });
      return;
    }
    active = result;
    activity = new PlaybackActivity();
    sequence = 0;
    position = result.position;
    subtitle = result.selected_subtitle ?? 'off';
    if (subtitle !== 'off') lastSubtitle = subtitle;
    await tick();
    if (revision !== generation) return;
    await attach(result.url, result.position);
  }
  async function attach(url: string, at: number, autoplay = true) {
    if (!active || !player) return;
    buffering = true;
    const session = active;
    const play = () => {
      if (active !== session) return;
      void player.play().catch(() => {
        if (active === session) {
          paused = true;
          ready();
        }
      });
    };
    player.volume = $appearance.audio_volume;
    player.playbackRate = speed;
    if (!active.video) {
      if (audioElement !== player) {
        await audioContext?.close();
        audioContext = new AudioContext();
        gain = audioContext.createGain();
        audioContext
          .createMediaElementSource(player)
          .connect(gain)
          .connect(audioContext.destination);
        audioElement = player;
      }
      gain!.gain.value = replayGain(active.probe, active.replay_gain);
      await audioContext?.resume();
    }
    hls?.destroy();
    hls = undefined;
    if (
      active.mode === 'direct' ||
      (!Hls.isSupported() &&
        player.canPlayType('application/vnd.apple.mpegurl'))
    ) {
      player.src = mediaUrl(url);
      player.onloadedmetadata = () => {
        if (active !== session) return;
        player.currentTime = Math.max(0, at - session.timeline_start);
        if (autoplay) play();
      };
    } else {
      hls = new Hls({
        enableWorker: false,
        startPosition: Math.max(0, at - active.timeline_start),
        liveSyncDurationCount: 1000,
        maxLiveSyncPlaybackRate: 1,
        maxBufferLength: 30,
      });
      hls.loadSource(mediaUrl(url));
      hls.attachMedia(player);
      if (autoplay) hls.on(Hls.Events.MANIFEST_PARSED, play);
      hls.on(Hls.Events.ERROR, (_, data) => {
        if (data.fatal && active === session) {
          error = 'Playback interrupted. Try seeking or reopening this item.';
          player.pause();
        }
      });
    }
  }
  function report(state: string) {
    if (!active) return Promise.resolve();
    const id = active.id;
    const data = {
      sequence: sequence++,
      position,
      state,
      active_seconds: activity.seconds(),
    };
    reporting = reporting
      .catch(() => {})
      .then(() => api(`/playback/${id}/progress`, 'POST', data))
      .catch((e) => {
        if (active?.id !== id) return;
        error = String(e);
        player?.pause();
      });
    return reporting;
  }
  async function stop() {
    activity.setActive(false);
    const stopped = active ? report('stopped') : Promise.resolve();
    active = null;
    paused = true;
    hls?.destroy();
    hls = undefined;
    if (player) {
      player.onloadedmetadata = null;
      player.pause();
      player.removeAttribute('src');
      player.load();
    }
    const closingAudio = audioContext?.close();
    audioContext = undefined;
    audioElement = undefined;
    await closingAudio;
    await stopped;
  }
  async function seek(at: number, resume?: boolean) {
    if (!active || active.live) return;
    const shouldResume = resume ?? !player.paused;
    if (active.mode === 'direct') {
      player.currentTime = at;
      position = at;
      await report(player.paused ? 'paused' : 'playing');
      return;
    }
    if (busy) return;
    busy = true;
    error = '';
    const revision = generation;
    const session = active;
    try {
      player.pause();
      const next = await api<{
        url: string;
        timeline_start: number;
        position: number;
      }>(`/playback/${active.id}/seek`, 'POST', {
        position: Math.min(at, active.duration - 0.05),
      });
      if (revision !== generation || active !== session) return;
      active = { ...active, ...next };
      selectSubtitle();
      position = next.position;
      await attach(next.url, next.position, shouldResume);
      await report(shouldResume ? 'playing' : 'paused');
    } catch (e) {
      if (revision === generation) error = String(e);
    } finally {
      if (revision === generation) busy = false;
    }
  }
  async function changeOptions() {
    const at = position;
    const revision = generation;
    busy = true;
    error = '';
    try {
      await stop();
      if (revision === generation) await start(at, revision);
    } catch (e) {
      if (revision === generation) error = String(e);
    } finally {
      if (revision === generation) busy = false;
    }
  }
  function selectSubtitle() {
    if (!player) return;
    for (const track of player.textTracks) {
      track.mode = track.id === subtitle ? 'showing' : 'disabled';
      for (const cue of track.cues ?? []) {
        let original = cueTimes.get(cue);
        if (!original) {
          original = { start: cue.startTime, end: cue.endTime };
          cueTimes.set(cue, original);
        }
        cue.startTime = Math.max(
          0,
          original.start - (active?.timeline_start ?? 0),
        );
        cue.endTime = Math.max(0, original.end - (active?.timeline_start ?? 0));
      }
    }
  }
  function update() {
    if (active && !busy)
      position = Math.min(
        active.live ? Number.POSITIVE_INFINITY : active.duration,
        player.currentTime + active.timeline_start,
      );
  }
  async function end() {
    activity.setActive(false);
    if (active) {
      if (!active.live) position = active.duration;
      await report('stopped');
      paused = true;
      ended?.();
    }
  }
  onDestroy(() => {
    generation++;
    clearInterval(timer);
    clearTimeout(hideTimer);
    void stop();
  });
  async function toggle() {
    if (!active || busy) return;
    const revision = generation;
    const session = active;
    if (active && !active.live && position >= active.duration - 0.1) {
      busy = true;
      try {
        await stop();
        if (revision === generation) await start(0, revision);
      } catch (e) {
        if (revision === generation) error = String(e);
      } finally {
        if (revision === generation) busy = false;
      }
      return;
    }
    if (!player.paused) {
      player.pause();
      return;
    }
    await audioContext?.resume();
    if (revision !== generation || active !== session) return;
    if (active.mode !== 'direct' && !active.live) await seek(position, true);
    else await player.play().catch((e) => (error = String(e)));
  }
</script>

<svelte:document
  onfullscreenchange={() =>
    (fullscreen = document.fullscreenElement === container)}
/>
<svelte:window
  bind:innerHeight={viewportHeight}
  onkeydown={(event) => {
    if (!container?.contains(event.target as Node | null)) return;
    revealControls();
    handleShortcut(event);
  }}
/>

<section
  id={playerId}
  data-sidebar-resize={resizable ? 'xy' : undefined}
  data-sidebar-resize-origin={resizable ? '' : undefined}
  bind:this={container}
  bind:clientHeight={playerHeight}
  style:--player-height={`${playerHeight}px`}
  style:height={fullscreen || boundedHeight === undefined
    ? undefined
    : `${boundedHeight}px`}
  transition:playerReveal|global
  onoutrostart={() => {
    generation++;
    player?.pause();
  }}
  class={[
    'player relative isolate aspect-video min-h-[210px] w-full overflow-hidden bg-[#090b0c] text-[#f5f6f7] [color-scheme:dark] [container-type:inline-size] fullscreen:m-0 fullscreen:aspect-auto fullscreen:h-full fullscreen:max-h-none fullscreen:w-full',
    resizable ? 'm-0' : 'mb-7',
    boundedHeight === undefined ? 'max-h-[60vh]' : 'max-h-[calc(100dvh-120px)]',
  ]}
  class:controls-hidden={!controlsShown}
  aria-label="Media player"
  tabindex="-1"
  onpointermove={revealControls}
  onpointerdown={(event) => {
    controlFocused = false;
    if (event.button === 0 && !isInteractiveTarget(event.target))
      container.focus({ preventScroll: true });
    revealControls();
  }}
  oncontextmenu={(event) => {
    if (isInteractiveTarget(event.target)) return;
    event.preventDefault();
    void toggle();
    revealControls();
  }}
  onfocusin={(event) => {
    controlFocused =
      event.target instanceof HTMLElement &&
      event.target.matches(':focus-visible');
    revealControls();
  }}
  onfocusout={(event) => {
    if (!container.contains(event.relatedTarget as Node | null))
      controlFocused = false;
  }}
>
  <video
    class="block h-full w-full object-contain"
    data-sidebar-resize="video"
    bind:this={player}
    playsinline
    ontimeupdate={update}
    onloadstart={waiting}
    onwaiting={waiting}
    onseeking={waiting}
    onseeked={ready}
    onloadeddata={ready}
    oncanplay={ready}
    onplaying={ready}
    onplay={() => {
      paused = false;
      ready();
    }}
    onpause={() => {
      activity.setActive(false);
      paused = true;
      void report('paused');
    }}
    onended={() => void end()}
    onerror={() => {
      if (active)
        error = 'Unable to play this media. Select Auto quality to convert it.';
    }}
  >
    {#each active?.subtitles ?? [] as track (track.id)}
      <track
        id={track.id}
        src={mediaUrl(track.url)}
        kind="subtitles"
        srclang={track.language || 'und'}
        label={track.title || track.language || 'Subtitles'}
        onload={selectSubtitle}
      />
    {/each}
  </video>

  <div
    class="player-header absolute inset-x-0 top-0 z-[2] flex items-center gap-4 bg-[linear-gradient(#000b,transparent)] px-4 pt-3 pb-9 opacity-100 transition-opacity duration-[180ms] ease-[ease] motion-reduce:transition-none max-[600px]:px-2 max-[600px]:pt-2 max-[600px]:pb-7"
  >
    <h2
      class="m-0 min-w-0 flex-1 overflow-hidden text-[15px] font-[550] text-ellipsis whitespace-nowrap [text-shadow:0_1px_4px_#000] max-[600px]:text-[13px]"
      data-sidebar-resize="xy-pos"
      title={choice.title}
    >
      {choice.title}
    </h2>
    <button
      data-sidebar-resize="xy-pos"
      class="player-button inline-grid size-9 shrink-0 place-items-center border-0 bg-transparent p-0 text-inherit transition-[background] duration-150 hover:bg-[#ffffff24]"
      aria-label="Close player"
      title="Close player"
      onclick={closed}
    >
      <X size={20} />
    </button>
  </div>

  {#if error}
    <div
      class="player-status pointer-events-none absolute inset-[68px_20px_92px] flex flex-col items-center justify-center gap-3 text-center [text-shadow:0_1px_4px_#000]"
      data-sidebar-resize="xy"
      data-sidebar-resize-origin
      role="alert"
    >
      <span class="inline-flex" data-sidebar-resize="xy-pos"
        ><AlertCircle size={28} /></span
      >
      <p
        class="m-0 max-w-120 wrap-anywhere text-[13px]"
        data-sidebar-resize="xy-pos"
      >
        {error}
      </p>
      <button
        class="retry-button pointer-events-auto border border-[#ffffff50] bg-[#15191be6] px-3 py-1.5 text-inherit"
        data-sidebar-resize="xy-pos"
        onclick={() => void open(choice)}>Retry playback</button
      >
    </div>
  {:else if loading}
    <div
      class="player-status pointer-events-none absolute inset-[68px_20px_92px] flex flex-col items-center justify-center gap-3 text-center [text-shadow:0_1px_4px_#000]"
      data-sidebar-resize="xy"
      data-sidebar-resize-origin
      role="status"
    >
      <span class="inline-flex" data-sidebar-resize="xy-pos"
        ><span
          class="loading-spinner inline-flex animate-spin text-accent motion-reduce:animate-none"
          ><LoaderCircle size={36} /></span
        ></span
      >
      <p
        class="m-0 max-w-120 wrap-anywhere text-[13px]"
        data-sidebar-resize="xy-pos"
      >
        {busy ? 'Preparing playback…' : 'Buffering…'}
      </p>
    </div>
  {/if}

  {#if active?.file_id && active.generation}
    <div
      class="segment-prompt absolute right-4 bottom-[110px]"
      data-sidebar-resize="xy-pos"
    >
      <SegmentSkip
        mediaId={choice.id}
        fileId={active.file_id}
        generation={active.generation}
        {position}
        {paused}
        busy={busy || buffering}
        {seek}
      />
    </div>
  {/if}

  <div
    class="player-controls absolute inset-x-0 bottom-0 z-[2] bg-[linear-gradient(transparent,#000c)] px-4 pt-[30px] pb-2.5 opacity-100 transition-opacity duration-[180ms] ease-[ease] motion-reduce:transition-none max-[600px]:px-2 max-[600px]:pt-6 max-[600px]:pb-1.5"
  >
    {#if !active?.live}
      <input
        class="seek block h-4 w-full cursor-pointer border-0 bg-transparent p-0 accent-accent"
        data-sidebar-resize="xy"
        aria-label="Playback position"
        aria-valuetext={`${time(position)} of ${time(active?.duration ?? 0)}`}
        type="range"
        min="0"
        max={active?.duration || 1}
        step="0.1"
        value={position}
        disabled={busy || !active}
        onchange={(event) => void seek(Number(event.currentTarget.value))}
      />
    {/if}
    <div
      class="control-row mt-[3px] flex items-center gap-2 max-[600px]:gap-0.5"
    >
      <button
        data-sidebar-resize="xy-pos"
        class="player-button inline-grid size-9 shrink-0 place-items-center border-0 bg-transparent p-0 text-inherit transition-[background] duration-150 hover:bg-[#ffffff24]"
        aria-label={paused ? 'Play' : 'Pause'}
        title={paused ? 'Play' : 'Pause'}
        disabled={busy || !active}
        onclick={() => void toggle()}
      >
        {#if paused}<Play size={20} fill="currentColor" />{:else}<Pause
            size={20}
            fill="currentColor"
          />{/if}
      </button>
      <div
        class="volume-controls flex items-center gap-2 max-[600px]:gap-0.5"
        data-sidebar-resize="xy-pos"
      >
        <button
          class="player-button inline-grid size-9 shrink-0 place-items-center border-0 bg-transparent p-0 text-inherit transition-[background] duration-150 hover:bg-[#ffffff24]"
          aria-label={muted || $appearance.audio_volume === 0
            ? 'Unmute'
            : 'Mute'}
          title={muted || $appearance.audio_volume === 0 ? 'Unmute' : 'Mute'}
          onclick={toggleMute}
        >
          {#if muted || $appearance.audio_volume === 0}<VolumeX
              size={20}
            />{:else}<Volume2 size={20} />{/if}
        </button>
        <input
          class="player-volume-range block h-4 w-[76px] cursor-pointer border-0 bg-transparent p-0 accent-accent max-[600px]:w-10"
          aria-label="Playback volume"
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={muted ? 0 : $appearance.audio_volume}
          oninput={(event) => changeVolume(Number(event.currentTarget.value))}
        />
      </div>
      <div
        class="playback-details ml-2 grid shrink-0 gap-0.5 max-[600px]:ml-1"
        data-sidebar-resize="xy-pos"
      >
        <span
          class="playback-time inline-flex items-center gap-[7px] text-xs whitespace-nowrap tabular-nums max-[600px]:text-[11px]"
          aria-live="off"
        >
          {#if active?.live}<span
              class="live-dot size-1.5 rounded-full bg-accent"
            ></span>Live{:else}{time(position)} / {time(
              active?.duration ?? 0,
            )}{/if}
        </span>
        {#if active}<span class="playback-mode text-[11px] text-[#acb4ba]"
            >{active.mode === 'direct'
              ? 'Original file'
              : active.mode === 'remux'
                ? 'Original codecs'
                : 'Converted'}{info?.watched ? ' · Watched' : ''}</span
          >{/if}
      </div>
      <div class="control-spacer flex-1"></div>
      <div
        class="player-options flex shrink-0 gap-0.5"
        data-sidebar-resize="xy-pos"
        role="group"
        aria-label="Playback settings"
      >
        <PlayerOption
          label="Quality"
          value={quality}
          options={[
            { value: 'auto', label: 'Auto' },
            { value: 'original', label: 'Original' },
            ...[2, 4, 8, 20].map((rate) => ({
              value: `${rate}mbps`,
              label: `${rate} Mbps`,
            })),
          ]}
          disabled={busy || !active}
          open={optionMenu === 'quality'}
          setOpen={(open) => (optionMenu = open ? 'quality' : null)}
          select={(value) => {
            quality = value;
            void changeOptions();
          }}
        >
          {#snippet icon()}<Settings2 size={20} />{/snippet}
        </PlayerOption>
        <PlayerOption
          label="Audio"
          value={audio === null ? 'default' : String(audio)}
          options={[
            { value: 'default', label: 'Default' },
            ...(
              active?.tracks.filter((track) => track.kind === 'audio') ?? []
            ).map((track) => ({
              value: String(track.index),
              label:
                `${track.language ?? ''} ${track.title ?? ''} (${track.codec})`.trim(),
            })),
          ]}
          disabled={busy || !active}
          open={optionMenu === 'audio'}
          setOpen={(open) => (optionMenu = open ? 'audio' : null)}
          select={(value) => {
            audio = value === 'default' ? null : Number(value);
            void changeOptions();
          }}
        >
          {#snippet icon()}<AudioLines size={20} />{/snippet}
        </PlayerOption>
        <PlayerOption
          label="Subtitles"
          value={subtitle}
          options={[
            { value: 'off', label: 'Off' },
            ...(active?.subtitles ?? []).map((track) => ({
              value: track.id,
              label:
                `${track.language ?? ''} ${track.title ?? ''}`.trim() ||
                'Subtitles',
            })),
          ]}
          disabled={busy || !active}
          open={optionMenu === 'subtitles'}
          setOpen={(open) => (optionMenu = open ? 'subtitles' : null)}
          select={setSubtitle}
        >
          {#snippet icon()}<Captions size={20} />{/snippet}
        </PlayerOption>
      </div>
      <button
        class="player-button inline-grid size-9 shrink-0 place-items-center border-0 bg-transparent p-0 text-inherit transition-[background] duration-150 hover:bg-[#ffffff24]"
        data-sidebar-resize="xy-pos"
        aria-label={fullscreen ? 'Exit fullscreen' : 'Fullscreen'}
        title={fullscreen ? 'Exit fullscreen' : 'Fullscreen'}
        onclick={() => void toggleFullscreen()}
      >
        {#if fullscreen}<Minimize size={20} />{:else}<Maximize size={20} />{/if}
      </button>
    </div>
  </div>
</section>

{#if resizable && !fullscreen}
  <div
    role="slider"
    tabindex="0"
    aria-label="Resize player height"
    aria-orientation="vertical"
    data-sidebar-resize="xy"
    aria-controls={playerId}
    aria-valuemin={minimumHeight}
    aria-valuemax={maximumHeight}
    aria-valuenow={Math.round(playerHeight)}
    aria-valuetext={`${Math.round(playerHeight)} pixels`}
    title="Drag to resize; use arrow keys or double-click to reset"
    class="player-resize-handle group flex h-2 w-full touch-none cursor-row-resize items-center justify-center bg-surface hover:bg-accent-soft focus-visible:bg-accent-soft focus-visible:-outline-offset-2"
    onpointerdown={(event) => {
      if (event.button !== 0 || !event.isPrimary) return;
      event.preventDefault();
      drag = {
        pointer: event.pointerId,
        y: event.clientY,
        height: playerHeight,
      };
      event.currentTarget.setPointerCapture(event.pointerId);
    }}
    onpointermove={(event) => {
      if (drag?.pointer === event.pointerId)
        resizeBy(drag.height + event.clientY - drag.y);
    }}
    onpointerup={(event) => {
      if (drag?.pointer === event.pointerId) {
        finishResize();
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    }}
    onlostpointercapture={finishResize}
    onpointercancel={finishResize}
    ondblclick={resetHeight}
    onkeydown={(event) => {
      const step = event.shiftKey ? 50 : 10;
      if (event.key === 'ArrowUp') resizeBy(playerHeight - step, true);
      else if (event.key === 'ArrowDown') resizeBy(playerHeight + step, true);
      else if (event.key === 'Home') resizeBy(minimumHeight, true);
      else if (event.key === 'End') resizeBy(maximumHeight, true);
      else if (event.key === 'Enter') resetHeight();
      else return;
      event.preventDefault();
    }}
  >
    <span
      class="h-0.5 w-12 rounded-full bg-line-strong group-hover:bg-accent group-focus-visible:bg-accent"
    ></span>
  </div>
{/if}

<style>
  :is(button, input):focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .controls-hidden {
    cursor: none;
  }
  .controls-hidden :is(.player-header, .player-controls) {
    opacity: 0;
    pointer-events: none;
  }
  @container (max-width: 380px) {
    .player-volume-range {
      display: none;
    }
    .playback-mode {
      display: none;
    }
  }
</style>
