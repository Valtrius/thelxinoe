<script lang="ts">
  import Button from './ui/Button.svelte';
  import { formControlClass } from './ui/styles';
  import { twMerge } from 'tailwind-merge';
  import { onDestroy, onMount, tick, untrack } from 'svelte';
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
  import { appearance, updateAppearance, videoMuted } from './appearance';
  import { compactViewport, touchDevice } from './responsive';
  import {
    capabilities,
    qualityOptions,
    mediaUrl,
    prepare,
    playbackRequest,
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
    music = false,
  } = $props<{
    choice: MediaChoice;
    closed: () => void;
    ended?: () => void;
    resizable?: boolean;
    /** Streamed music keeps the music volume and ignores device rotation. */
    music?: boolean;
  }>();
  const playerId = $props.id();
  const canResize = $derived(resizable && !compactViewport.current);
  let videoAspectRatio = $state(16 / 9);
  let videoHeight = $state(0);
  const deliveredQuality = $derived.by(() => {
    if (!active?.video) return undefined;
    if (active.quality) return active.quality;
    const stream = active.probe.streams?.find((s) => s.codec_type === 'video');
    const height = videoHeight || stream?.height;
    const fps =
      parseFrameRate(stream?.avg_frame_rate) ??
      parseFrameRate(stream?.r_frame_rate);
    return height
      ? `${height}p${fps && fps > 30 ? Math.round(fps) : ''}`
      : undefined;
  });
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
    waitingTools = $state(false),
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
    nativeFullscreen = $state(false),
    viewportFullscreen = $state(false),
    speed = $state(1);
  // Phones and tablets play video at the device volume. Streamed music keeps
  // its own mute instead of changing the remembered video mute.
  const deviceVolume = $derived(touchDevice.current && !music);
  let musicMuted = $state(false);
  const muted = $derived(!deviceVolume && (music ? musicMuted : $videoMuted));
  function setMuted(value: boolean) {
    if (music) musicMuted = value;
    else videoMuted.set(value);
  }
  const volume = $derived(deviceVolume ? 1 : $appearance.audio_volume);
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  const fullscreen = $derived(nativeFullscreen || viewportFullscreen);
  let rotationFullscreen = false;
  let fullscreenRevision = 0;
  let tapTimer: ReturnType<typeof setTimeout> | undefined;
  let sideTap: { side: -1 | 1; time: number; seeking: boolean } | undefined;
  let tapSeek:
    { revision: number; target: number; resume: boolean } | undefined;
  // The seek bar keeps the value under the pointer until it is released;
  // playback updates would otherwise replace it and seek back on release.
  // Releasing where the drag began fires no change event, so release clears it.
  let scrub = $state<number | null>(null);
  // Touch cannot hover: a tap over hidden controls only reveals them, and the
  // :hover it leaves on a tapped control must not keep them shown.
  let touch = false,
    tapRevealsControls = false;
  let audibleVolume = 1;
  let lastSubtitle: string | null = null;
  $effect.pre(() => {
    const savedHeight = $appearance.player_height;
    if (canResize && !drag) resizedHeight = savedHeight;
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
      player.volume = volume;
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
        touch ||
        !container?.querySelector(
          '.player-header:hover, .player-controls:hover',
        )
      )
        controlsVisible = false;
    }, 2500);
  }
  function changeVolume(value: number) {
    if (deviceVolume) return;
    value = Math.max(0, Math.min(1, value));
    setMuted(false);
    player.volume = value;
    updateAppearance({ audio_volume: value });
  }
  function toggleMute() {
    if (deviceVolume) return;
    if ($appearance.audio_volume === 0) changeVolume(audibleVolume);
    else setMuted(!muted);
  }
  async function enterFullscreen(rotated = false) {
    const revision = ++fullscreenRevision;
    rotationFullscreen = rotated;
    if (
      rotated &&
      (!navigator.userActivation?.isActive || !document.fullscreenEnabled)
    ) {
      viewportFullscreen = true;
      return;
    }
    try {
      await container.requestFullscreen();
      if (
        revision !== fullscreenRevision &&
        document.fullscreenElement === container
      )
        await document.exitFullscreen();
    } catch {
      if (
        touchDevice.current &&
        revision === fullscreenRevision &&
        container.isConnected
      )
        viewportFullscreen = true;
    }
    if (!rotated) revealControls();
  }
  async function exitFullscreen(rotated = false) {
    fullscreenRevision++;
    viewportFullscreen = false;
    if (document.fullscreenElement === container)
      await document.exitFullscreen().catch(() => {});
    rotationFullscreen = false;
    if (!rotated) revealControls();
  }
  async function toggleFullscreen() {
    if (viewportFullscreen || document.fullscreenElement === container)
      await exitFullscreen();
    else await enterFullscreen();
  }
  $effect(() => {
    if (
      viewportFullscreen &&
      container &&
      !container.matches(':popover-open')
    ) {
      container.showPopover();
      if (!rotationFullscreen) container.focus({ preventScroll: true });
    }
  });
  onMount(() => {
    const landscape = () =>
      screen.orientation
        ? screen.orientation.type.startsWith('landscape')
        : Math.abs(Number(window.orientation)) === 90;
    let previousLandscape = landscape();
    const rotate = () => {
      const next = landscape();
      const changed = next !== previousLandscape;
      previousLandscape = next;
      if (!touchDevice.current || music || !changed) return;
      if (next && !fullscreen) void enterFullscreen(true);
      else if (!next) void exitFullscreen(true);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || !viewportFullscreen) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      void exitFullscreen();
    };
    screen.orientation?.addEventListener('change', rotate);
    window.addEventListener('orientationchange', rotate);
    window.addEventListener('keydown', escape, true);
    return () => {
      fullscreenRevision++;
      screen.orientation?.removeEventListener('change', rotate);
      window.removeEventListener('orientationchange', rotate);
      window.removeEventListener('keydown', escape, true);
    };
  });
  function clearSideTap() {
    clearTimeout(tapTimer);
    tapTimer = undefined;
    sideTap = undefined;
  }
  // The previous shape is kept while the next item loads; audio-only streams
  // have no dimensions and use the default video shape.
  function updateAspectRatio() {
    videoHeight = player.videoHeight;
    videoAspectRatio =
      player.videoWidth > 0 && player.videoHeight > 0
        ? player.videoWidth / player.videoHeight
        : 16 / 9;
  }
  function tapVideo(event: MouseEvent) {
    const revealOnly = tapRevealsControls;
    tapRevealsControls = false;
    revealControls();
    if (!touchDevice.current || !touch) {
      if (!revealOnly) void toggle();
      return;
    }
    const bounds = player.getBoundingClientRect();
    const fraction = (event.clientX - bounds.left) / bounds.width;
    if (fraction >= 1 / 3 && fraction <= 2 / 3) {
      clearSideTap();
      if (!revealOnly) void toggle();
      return;
    }
    const side = fraction < 1 / 3 ? -1 : 1;
    const now = performance.now();
    const repeat = sideTap?.side === side && now - sideTap.time <= 300;
    clearTimeout(tapTimer);
    sideTap = { side, time: now, seeking: repeat };
    if (repeat) void seekByTap(side * 10);
    tapTimer = setTimeout(() => {
      const pause = !sideTap?.seeking && !revealOnly;
      clearSideTap();
      if (pause) void toggle();
    }, 300);
  }
  async function seekByTap(seconds: number) {
    if (!active || active.live) return;
    const pending = tapSeek?.revision === generation ? tapSeek : undefined;
    if (busy && !pending) return;
    const target = Math.max(
      0,
      Math.min(active.duration, (pending?.target ?? position) + seconds),
    );
    if (pending) {
      pending.target = target;
      return;
    }
    const request = {
      revision: generation,
      target,
      resume: holding || !player.paused,
    };
    tapSeek = request;
    try {
      while (tapSeek === request && request.revision === generation) {
        const target = request.target;
        await seek(target, request.resume);
        if (target === request.target) break;
      }
    } finally {
      if (tapSeek === request) tapSeek = undefined;
    }
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
        if (deviceVolume) return;
        changeVolume($appearance.audio_volume + 0.05);
        break;
      case 'ArrowDown':
        if (deviceVolume) return;
        changeVolume($appearance.audio_volume - 0.05);
        break;
      case 'm':
        if (deviceVolume) return;
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
    if (holding) {
      release();
      if (holding) return;
    }
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
  // A seek into media that is not buffered yet waits for enough to continue,
  // instead of resuming and stalling again at the end of a long fragment.
  const resumeBuffer = 3;
  let holding = false;
  function bufferedAhead() {
    const at = player.currentTime;
    for (let index = 0; index < player.buffered.length; index++)
      if (
        player.buffered.start(index) <= at + 0.25 &&
        at < player.buffered.end(index)
      )
        return player.buffered.end(index) - at;
    return 0;
  }
  function hold() {
    holding = true;
    buffering = true;
    player.pause();
  }
  function release() {
    if (!holding || !player || player.seeking) return;
    const ahead = bufferedAhead();
    const end = Number.isFinite(player.duration)
      ? player.duration
      : Number.POSITIVE_INFINITY;
    if (ahead < resumeBuffer && player.currentTime + ahead < end - 0.25) return;
    holding = false;
    void player.play().catch(() => {
      // Another seek interrupted this request and resumes when it can.
      if (holding) return;
      paused = true;
      ready();
    });
  }
  let activity = new PlaybackActivity();
  let stalledSince: number | undefined;
  let lastPlaybackTime = 0;
  const stallTimer = setInterval(() => {
    release();
    if (
      !active ||
      busy ||
      !buffering ||
      (player.paused && !holding) ||
      error ||
      player.ended
    ) {
      stalledSince = undefined;
      lastPlaybackTime = player?.currentTime ?? 0;
      return;
    }
    if (player.currentTime !== lastPlaybackTime) stalledSince = undefined;
    lastPlaybackTime = player.currentTime;
    stalledSince ??= performance.now();
    if (performance.now() - stalledSince >= 30000) {
      error =
        'The stream stopped supplying media. Try seeking or reopening this item.';
      holding = false;
      player.pause();
    }
  }, 1000);
  let preparation = new AbortController();
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
    container.focus({ preventScroll: true });
    preparation.abort();
    preparation = new AbortController();
    waitingTools = false;
    clearSideTap();
    tapSeek = undefined;
    const revision = ++generation;
    busy = true;
    buffering = true;
    optionMenu = null;
    info = null;
    error = '';
    await stop();
    if (revision !== generation) return;
    position = 0;
    videoHeight = 0;
    const pending = {
      signal: preparation.signal,
      waiting: (value: boolean) => {
        if (revision === generation) waitingTools = value;
      },
    };
    try {
      if (current.resolveYoutube)
        await playbackRequest(
          () =>
            api('/online/youtube/resolve', 'POST', {
              video_id: current.id.slice('youtube:'.length),
            }),
          pending,
        );
      if (revision !== generation) return;
      const loaded = await playbackRequest(
        () => api<MediaInfo>(`/catalog/${current.id}/playback`),
        pending,
      );
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
  async function start(at?: number, revision = generation, autoplay = true) {
    const result = await prepare(
      choice,
      {
        quality,
        audio,
        subtitle: subtitle || null,
        capabilities: capabilities(
          info?.sources.find((s) => !choice.fileId || s.id === choice.fileId)
            ?.probe,
        ),
      },
      at,
      {
        signal: preparation.signal,
        waiting: (value) => {
          if (revision === generation) waitingTools = value;
        },
      },
    );
    if (revision === generation && active) await stop();
    if (revision !== generation) {
      await api(`/playback/${result.id}/progress`, 'POST', {
        sequence: 0,
        position: result.position,
        state: 'stopped',
      });
      return;
    }
    active = result;
    if (result.qualities?.length) {
      if (quality.endsWith('mbps')) quality = 'auto';
      else if (
        quality !== 'auto' &&
        !result.qualities.some((item) => item.value === quality)
      )
        quality = result.quality ?? 'auto';
    }
    activity = new PlaybackActivity();
    sequence = 0;
    position = result.position;
    subtitle = result.selected_subtitle ?? 'off';
    if (subtitle !== 'off') lastSubtitle = subtitle;
    await tick();
    if (revision !== generation) return;
    await attach(result.url, result.position, autoplay);
  }
  async function attach(url: string, at: number, autoplay = true) {
    if (!active || !player) return;
    buffering = true;
    const session = active;
    const play = () => {
      if (active !== session) return;
      void player.play().catch(() => {
        // A seek before playback starts interrupts this request; it resumes
        // when the seek has enough media.
        if (active === session && !holding) {
          paused = true;
          ready();
        }
      });
    };
    player.volume = volume;
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
    // Online videos play their original files as HLS, even in direct mode.
    const playlist = new URL(url, location.href).pathname.endsWith('.m3u8');
    if (
      !playlist ||
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
      hls.on(Hls.Events.FRAG_BUFFERED, release);
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
    holding = false;
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
    const shouldResume = resume ?? (holding || !player.paused);
    if (active.mode === 'direct') {
      player.currentTime = at;
      position = at;
      if (shouldResume && hls && bufferedAhead() < resumeBuffer) hold();
      await report(shouldResume ? 'playing' : 'paused');
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
    const autoplay = holding || !player.paused;
    const revision = generation;
    busy = true;
    error = '';
    try {
      player.pause();
      if (revision === generation) await start(at, revision, autoplay);
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
    preparation.abort();
    generation++;
    clearInterval(timer);
    clearInterval(stallTimer);
    clearTimeout(hideTimer);
    clearSideTap();
    void stop();
  });
  async function toggle() {
    if (!active || busy) return;
    if (holding) {
      // Pausing while a seek waits for media keeps playback paused.
      holding = false;
      paused = true;
      activity.setActive(false);
      void report('paused');
      return;
    }
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
    const buffered = Array.from(
      { length: player.buffered.length },
      (_, i) =>
        player.currentTime >= player.buffered.start(i) &&
        player.currentTime < player.buffered.end(i),
    );
    const firstFragment = hls?.latestLevelDetails?.fragments[0];
    const expired = firstFragment && player.currentTime < firstFragment.start;
    if (
      active.mode !== 'direct' &&
      !active.live &&
      expired &&
      !buffered.some(Boolean)
    )
      await seek(position, true);
    else await player.play().catch((e) => (error = String(e)));
  }
</script>

<svelte:document
  onfullscreenchange={() =>
    (nativeFullscreen = document.fullscreenElement === container)}
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
  popover={viewportFullscreen ? 'manual' : undefined}
  data-sidebar-resize={canResize ? 'xy' : undefined}
  data-sidebar-resize-origin={canResize ? '' : undefined}
  bind:this={container}
  bind:clientHeight={playerHeight}
  style:--player-height={`${playerHeight}px`}
  style:height={compactViewport.current ||
  fullscreen ||
  boundedHeight === undefined
    ? undefined
    : `${boundedHeight}px`}
  style:aspect-ratio={compactViewport.current && !fullscreen
    ? videoAspectRatio
    : undefined}
  style:max-height={compactViewport.current && !fullscreen
    ? '60dvh'
    : undefined}
  transition:playerReveal|global
  onoutrostart={() => {
    preparation.abort();
    generation++;
    fullscreenRevision++;
    clearSideTap();
    tapSeek = undefined;
    player?.pause();
  }}
  class={twMerge([
    'player relative isolate aspect-video min-h-[210px] w-full overflow-hidden bg-[#090b0c] text-[#f5f6f7] [color-scheme:dark] @container/player compact:min-h-0 compact:max-h-none fullscreen:m-0 fullscreen:aspect-auto fullscreen:h-full fullscreen:max-h-none fullscreen:w-full',
    resizable ? 'm-0' : 'mb-7',
    fullscreen && 'outline-none',
    !controlsShown && 'controls-hidden cursor-none',
    boundedHeight === undefined ? 'max-h-[60vh]' : 'max-h-[calc(100dvh-120px)]',
    viewportFullscreen &&
      'fixed inset-0 z-[100] m-0 aspect-auto h-dvh max-h-none w-screen border-0 p-0',
  ])}
  aria-label="Media player"
  tabindex="-1"
  onpointermove={(event) => {
    touch = event.pointerType !== 'mouse';
    revealControls();
  }}
  onpointerdown={(event) => {
    touch = event.pointerType !== 'mouse';
    tapRevealsControls = touch && !controlsShown;
    controlFocused = false;
    if (isInteractiveTarget(event.target)) clearSideTap();
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
    if (rotationFullscreen && event.target === container) return;
    controlFocused =
      event.target !== container &&
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
    class="block h-full w-full touch-manipulation object-contain"
    data-sidebar-resize="video"
    bind:this={player}
    playsinline
    onclick={tapVideo}
    ontimeupdate={update}
    onloadstart={waiting}
    onwaiting={waiting}
    onseeking={waiting}
    onseeked={ready}
    onloadeddata={ready}
    onloadedmetadata={updateAspectRatio}
    onresize={updateAspectRatio}
    oncanplay={ready}
    onplaying={ready}
    onplay={() => {
      paused = false;
      ready();
    }}
    onpause={() => {
      if (holding) return;
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
    class={[
      'player-header absolute inset-x-0 top-0 z-[2] flex items-center gap-4 bg-[linear-gradient(#000b,transparent)] px-4 pt-3 pb-9 transition-opacity duration-(--animation-duration) ease-[ease] motion-reduce:transition-none max-[600px]:px-2 max-[600px]:pt-2 max-[600px]:pb-7',
      !controlsShown && 'pointer-events-none opacity-0',
    ]}
  >
    <h2
      class="m-0 min-w-0 flex-1 overflow-hidden text-[15px] font-[550] text-ellipsis whitespace-nowrap [text-shadow:0_1px_4px_#000] max-[600px]:text-[13px]"
      data-sidebar-resize="xy-pos"
      title={choice.title}
    >
      {choice.title}
    </h2>
    <Button
      size="icon"
      variant="player"
      data-sidebar-resize="xy-pos"
      class="player-button shrink-0"
      aria-label="Close player"
      title="Close player"
      onclick={closed}
    >
      <X size={20} />
    </Button>
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
        class="retry-button focus-visible:outline-2 focus-visible:outline-offset-2 pointer-events-auto border border-[#ffffff50] bg-[#15191be6] px-3 py-1.5 text-inherit"
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
        {waitingTools
          ? 'Waiting for playback tools…'
          : busy
            ? 'Preparing playback…'
            : 'Buffering…'}
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
    class={[
      'player-controls absolute inset-x-0 bottom-0 z-[2] bg-[linear-gradient(transparent,#000c)] px-4 pt-[30px] pb-2.5 transition-opacity duration-(--animation-duration) ease-[ease] motion-reduce:transition-none max-[600px]:px-2 max-[600px]:pt-6 max-[600px]:pb-1.5',
      !controlsShown && 'pointer-events-none opacity-0',
    ]}
  >
    {#if !active?.live}
      <input
        class={twMerge(
          formControlClass,
          'seek focus-visible:outline-2 focus-visible:outline-offset-2 block h-4 w-full cursor-pointer border-0 bg-transparent p-0 accent-accent',
        )}
        data-sidebar-resize="xy"
        aria-label="Playback position"
        aria-valuetext={`${time(scrub ?? position)} of ${time(active?.duration ?? 0)}`}
        type="range"
        min="0"
        max={active?.duration || 1}
        step="0.1"
        value={scrub ?? position}
        disabled={busy || !active}
        oninput={(event) => (scrub = Number(event.currentTarget.value))}
        onchange={(event) => {
          scrub = null;
          void seek(Number(event.currentTarget.value));
        }}
        onpointerup={() => setTimeout(() => (scrub = null))}
        onpointercancel={() => (scrub = null)}
      />
    {/if}
    <div
      class="control-row mt-[3px] flex items-center gap-2 max-[600px]:gap-0.5"
    >
      <Button
        size="icon"
        variant="player"
        data-sidebar-resize="xy-pos"
        class="player-button shrink-0"
        aria-label={paused ? 'Play' : 'Pause'}
        title={paused ? 'Play' : 'Pause'}
        disabled={busy || !active}
        onclick={() => void toggle()}
      >
        {#if paused}<Play size={20} fill="currentColor" />{:else}<Pause
            size={20}
            fill="currentColor"
          />{/if}
      </Button>
      {#if !deviceVolume}<div
          class="volume-controls flex items-center gap-2 max-[600px]:gap-0.5"
          data-sidebar-resize="xy-pos"
        >
          <Button
            size="icon"
            variant="player"
            class="player-button shrink-0"
            aria-label={muted || $appearance.audio_volume === 0
              ? 'Unmute'
              : 'Mute'}
            title={muted || $appearance.audio_volume === 0 ? 'Unmute' : 'Mute'}
            onclick={toggleMute}
          >
            {#if muted || $appearance.audio_volume === 0}<VolumeX
                size={20}
              />{:else}<Volume2 size={20} />{/if}
          </Button>
          <input
            class={twMerge(
              formControlClass,
              'player-volume-range focus-visible:outline-2 focus-visible:outline-offset-2 player-small:hidden block h-4 w-[76px] cursor-pointer border-0 bg-transparent p-0 accent-accent max-[600px]:w-10',
            )}
            aria-label="Playback volume"
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={muted ? 0 : $appearance.audio_volume}
            oninput={(event) => changeVolume(Number(event.currentTarget.value))}
          />
        </div>{/if}
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
        {#if active}<span
            class="playback-mode text-[11px] text-[#acb4ba] player-small:hidden"
            title={active.conversion_reason ?? undefined}
            >{deliveredQuality ? `${deliveredQuality} · ` : ''}{active.mode ===
            'direct'
              ? 'Original'
              : active.mode === 'remux'
                ? 'Remuxed'
                : active.encoding?.video !== false
                  ? 'Video converted'
                  : 'Audio converted'}{info?.watched ? ' · Watched' : ''}</span
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
          options={qualityOptions(active, info, quality, deliveredQuality)}
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
      <Button
        size="icon"
        variant="player"
        class="player-button shrink-0"
        data-sidebar-resize="xy-pos"
        aria-label={fullscreen ? 'Exit fullscreen' : 'Fullscreen'}
        title={fullscreen ? 'Exit fullscreen' : 'Fullscreen'}
        onclick={() => void toggleFullscreen()}
      >
        {#if fullscreen}<Minimize size={20} />{:else}<Maximize size={20} />{/if}
      </Button>
    </div>
  </div>
</section>

{#if canResize && !fullscreen}
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
