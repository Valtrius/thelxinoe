<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
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
  let { choice, closed, ended } = $props<{
    choice: MediaChoice;
    closed: () => void;
    ended?: () => void;
  }>();
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
    muted = $state(false);
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let audibleVolume = 1;
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
  function ready() {
    if (active && player.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA)
      buffering = false;
  }
  const timer = setInterval(() => {
    void report(paused ? 'paused' : 'playing');
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
    sequence = 0;
    position = result.position;
    subtitle = result.selected_subtitle ?? 'off';
    await tick();
    if (revision !== generation) return;
    await attach(result.url, result.position);
  }
  async function attach(url: string, at: number) {
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
        play();
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
      hls.on(Hls.Events.MANIFEST_PARSED, play);
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
    const data = { sequence: sequence++, position, state };
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
  async function seek(at: number) {
    if (!active || active.live || busy) return;
    busy = true;
    error = '';
    const revision = generation;
    const session = active;
    try {
      if (active.mode === 'direct') {
        player.currentTime = at;
        position = at;
        await report(paused ? 'paused' : 'playing');
      } else {
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
        await attach(next.url, next.position);
        await report('playing');
      }
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
    if (!paused) {
      player.pause();
      return;
    }
    await audioContext?.resume();
    if (revision !== generation || active !== session) return;
    if (active.mode !== 'direct' && !active.live) await seek(position);
    else await player.play().catch((e) => (error = String(e)));
  }
</script>

<svelte:document
  onfullscreenchange={() =>
    (fullscreen = document.fullscreenElement === container)}
/>
<svelte:window
  onkeydown={(event) => {
    if (!container?.contains(event.target as Node | null)) return;
    revealControls();
  }}
/>

<section
  bind:this={container}
  bind:clientHeight={playerHeight}
  style:--player-height={`${playerHeight}px`}
  transition:playerReveal|global
  onoutrostart={() => {
    generation++;
    player?.pause();
  }}
  class="player"
  class:controls-hidden={!controlsShown}
  aria-label="Media player"
  onpointermove={revealControls}
  onpointerdown={() => {
    controlFocused = false;
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
    data-sidebar-resize="video"
    bind:this={player}
    playsinline
    ontimeupdate={update}
    onloadstart={() => {
      if (active) buffering = true;
    }}
    onwaiting={() => {
      if (active) buffering = true;
    }}
    onseeking={() => {
      if (active) buffering = true;
    }}
    onseeked={ready}
    onloadeddata={ready}
    oncanplay={ready}
    onplaying={ready}
    onplay={() => {
      paused = false;
      void report('playing');
    }}
    onpause={() => {
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

  <div class="player-header">
    <h2 data-sidebar-resize="xy-pos" title={choice.title}>{choice.title}</h2>
    <button
      data-sidebar-resize="xy-pos"
      class="player-button"
      aria-label="Close player"
      title="Close player"
      onclick={closed}
    >
      <X size={20} />
    </button>
  </div>

  {#if error}
    <div
      class="player-status"
      data-sidebar-resize="xy"
      data-sidebar-resize-origin
      role="alert"
    >
      <span class="inline-flex" data-sidebar-resize="xy-pos"
        ><AlertCircle size={28} /></span
      >
      <p data-sidebar-resize="xy-pos">{error}</p>
      <button
        class="retry-button"
        data-sidebar-resize="xy-pos"
        onclick={() => void open(choice)}>Retry playback</button
      >
    </div>
  {:else if loading}
    <div
      class="player-status"
      data-sidebar-resize="xy"
      data-sidebar-resize-origin
      role="status"
    >
      <span class="inline-flex" data-sidebar-resize="xy-pos"
        ><span class="loading-spinner"><LoaderCircle size={36} /></span></span
      >
      <p data-sidebar-resize="xy-pos">
        {busy ? 'Preparing playback…' : 'Buffering…'}
      </p>
    </div>
  {/if}

  {#if active?.file_id && active.generation}
    <div class="segment-prompt" data-sidebar-resize="xy-pos">
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

  <div class="player-controls">
    {#if !active?.live}
      <input
        class="seek"
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
    <div class="control-row">
      <button
        data-sidebar-resize="xy-pos"
        class="player-button"
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
      <div class="volume-controls" data-sidebar-resize="xy-pos">
        <button
          class="player-button"
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
          aria-label="Playback volume"
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={muted ? 0 : $appearance.audio_volume}
          oninput={(event) => changeVolume(Number(event.currentTarget.value))}
        />
      </div>
      <div class="playback-details" data-sidebar-resize="xy-pos">
        <span class="playback-time" aria-live="off">
          {#if active?.live}<span class="live-dot"></span>Live{:else}{time(
              position,
            )} / {time(active?.duration ?? 0)}{/if}
        </span>
        {#if active}<span class="playback-mode"
            >{active.mode === 'direct'
              ? 'Original file'
              : active.mode === 'remux'
                ? 'Original codecs'
                : 'Converted'}{info?.watched ? ' · Watched' : ''}</span
          >{/if}
      </div>
      <div class="control-spacer"></div>
      <div
        class="player-options"
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
          select={(value) => {
            subtitle = value;
            selectSubtitle();
          }}
        >
          {#snippet icon()}<Captions size={20} />{/snippet}
        </PlayerOption>
      </div>
      <button
        class="player-button"
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

<style>
  .player {
    position: relative;
    isolation: isolate;
    container-type: inline-size;
    width: 100%;
    aspect-ratio: 16 / 9;
    max-height: 60vh;
    min-height: 210px;
    margin: 0 0 28px;
    overflow: hidden;
    background: #090b0c;
    color: #f5f6f7;
    color-scheme: dark;
  }
  .player:fullscreen {
    width: 100%;
    height: 100%;
    max-height: none;
    margin: 0;
    aspect-ratio: auto;
  }
  video {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .player-header,
  .player-controls {
    position: absolute;
    left: 0;
    right: 0;
    z-index: 2;
    opacity: 1;
    transition: opacity 180ms ease;
  }
  .player-header {
    top: 0;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 12px 16px 36px;
    background: linear-gradient(#000b, transparent);
  }
  .player-header h2 {
    flex: 1;
    min-width: 0;
    margin: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 15px;
    font-weight: 550;
    text-shadow: 0 1px 4px #000;
  }
  .player-button {
    display: inline-grid;
    place-items: center;
    flex-shrink: 0;
    width: 36px;
    height: 36px;
    padding: 0;
    border: 0;
    color: inherit;
    background: transparent;
    transition: background 150ms;
  }
  .player-button:hover {
    background: #ffffff24;
  }
  :is(button, input):focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .player-controls {
    bottom: 0;
    padding: 30px 16px 10px;
    background: linear-gradient(transparent, #000c);
  }
  .control-row,
  .volume-controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .control-row {
    margin-top: 3px;
  }
  .control-spacer {
    flex: 1;
  }
  input[type='range'] {
    display: block;
    height: 16px;
    padding: 0;
    border: 0;
    background: transparent;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .seek {
    width: 100%;
  }
  .volume-controls input {
    width: 76px;
  }
  .playback-details {
    display: grid;
    flex-shrink: 0;
    gap: 2px;
    margin-left: 8px;
  }
  .playback-time {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .live-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .controls-hidden {
    cursor: none;
  }
  .controls-hidden :is(.player-header, .player-controls) {
    opacity: 0;
    pointer-events: none;
  }
  .player-status {
    position: absolute;
    inset: 68px 20px 92px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-direction: column;
    gap: 12px;
    text-align: center;
    pointer-events: none;
    text-shadow: 0 1px 4px #000;
  }
  .player-status p {
    max-width: 480px;
    margin: 0;
    font-size: 13px;
    overflow-wrap: anywhere;
  }
  .loading-spinner {
    display: inline-flex;
    color: var(--accent);
    animation: player-spin 1s linear infinite;
  }
  .retry-button {
    padding: 6px 12px;
    border: 1px solid #ffffff50;
    color: inherit;
    background: #15191be6;
    pointer-events: auto;
  }
  .segment-prompt {
    position: absolute;
    right: 16px;
    bottom: 110px;
  }
  .player-options {
    display: flex;
    flex-shrink: 0;
    gap: 2px;
  }
  .playback-mode {
    color: #acb4ba;
    font-size: 11px;
  }
  @keyframes player-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (max-width: 600px) {
    .player-header {
      padding: 8px 8px 28px;
    }
    .player-header h2 {
      font-size: 13px;
    }
    .player-controls {
      padding: 24px 8px 6px;
    }
    .control-row,
    .volume-controls {
      gap: 2px;
    }
    .volume-controls input {
      width: 40px;
    }
    .playback-details {
      margin-left: 4px;
    }
    .playback-time {
      font-size: 11px;
    }
  }
  @container (max-width: 380px) {
    .volume-controls input {
      display: none;
    }
    .playback-mode {
      display: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .player-header,
    .player-controls {
      transition: none;
    }
    .loading-spinner {
      animation: none;
    }
  }
</style>
