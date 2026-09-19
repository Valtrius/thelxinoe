<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
  import Hls from 'hls.js';
  import SegmentSkip from './SegmentSkip.svelte';
  import { api } from './api';
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
  let active = $state<Playback | null>(null),
    info = $state<MediaInfo | null>(null),
    error = $state(''),
    busy = $state(false),
    paused = $state(true),
    position = $state(0),
    quality = $state('auto'),
    audio = $state<number | null>(null),
    subtitle = $state('off');
  let hls: Hls | undefined,
    sequence = 0,
    generation = 0,
    reporting: Promise<unknown> = Promise.resolve();
  let audioContext: AudioContext | undefined,
    audioElement: HTMLVideoElement | undefined,
    gain: GainNode | undefined;
  const cueTimes = new WeakMap<TextTrackCue, { start: number; end: number }>();
  const timer = setInterval(() => {
    void report(paused ? 'paused' : 'playing');
  }, 10000);
  $effect(() => {
    const current = choice;
    untrack(() => void open(current));
  });
  async function open(current: MediaChoice) {
    const revision = ++generation;
    await stop();
    busy = true;
    error = '';
    try {
      const loaded = await api<MediaInfo>(`/catalog/${current.id}/playback`);
      if (revision !== generation) return;
      info = loaded;
      quality = loaded.preferences.quality;
      audio = null;
      subtitle = loaded.preferences.subtitles ? '' : 'off';
      await start(undefined, revision);
    } catch (e) {
      error = String(e);
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
    await attach(result.url, result.position);
  }
  async function attach(url: string, at: number) {
    if (!active || !player) return;
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
        player.currentTime = Math.max(0, at - (active?.timeline_start ?? 0));
        void player.play().catch(() => (paused = true));
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
      hls.on(Hls.Events.MANIFEST_PARSED, () => {
        void player.play().catch(() => (paused = true));
      });
      hls.on(Hls.Events.ERROR, (_, data) => {
        if (data.fatal) {
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
        error = String(e);
        player?.pause();
      });
    return reporting;
  }
  async function stop() {
    if (active) {
      await report('stopped');
      active = null;
    }
    hls?.destroy();
    hls = undefined;
    if (player) {
      player.pause();
      player.removeAttribute('src');
      player.load();
    }
    await audioContext?.close();
    audioContext = undefined;
    audioElement = undefined;
  }
  async function seek(at: number) {
    if (!active || active.live || busy) return;
    busy = true;
    error = '';
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
        active = { ...active, ...next };
        selectSubtitle();
        position = next.position;
        await attach(next.url, next.position);
        await report('playing');
      }
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function changeOptions() {
    const at = position;
    busy = true;
    error = '';
    try {
      await stop();
      await start(at);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
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
    void stop();
  });
  async function toggle() {
    if (active && !active.live && position >= active.duration - 0.1) {
      busy = true;
      try {
        await stop();
        await start(0);
      } catch (e) {
        error = String(e);
      } finally {
        busy = false;
      }
      return;
    }
    if (!paused) {
      player.pause();
      return;
    }
    await audioContext?.resume();
    if (active?.mode !== 'direct') await seek(position);
    else await player.play().catch((e) => (error = String(e)));
  }
</script>

<section class="player panel" aria-label="Media player">
  <div class="section-heading">
    <h2>{choice.title}</h2>
    <button
      class="secondary"
      onclick={async () => {
        await stop();
        closed();
      }}>Close player</button
    >
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if busy}<p role="status">Preparing playback…</p>{/if}
  {#if active}
    <video
      bind:this={player}
      class:music={!active.video}
      playsinline
      ontimeupdate={update}
      onplay={() => {
        paused = false;
        void report('playing');
      }}
      onpause={() => {
        paused = true;
        void report('paused');
      }}
      onended={() => void end()}
      onerror={() =>
        (error =
          'Unable to play this media. Select Auto quality to convert it.')}
    >
      {#each active.subtitles as track (track.id)}<track
          id={track.id}
          src={mediaUrl(track.url)}
          kind="subtitles"
          srclang={track.language || 'und'}
          label={track.title || track.language || 'Subtitles'}
          onload={selectSubtitle}
        />{/each}
    </video>
    {#if active.file_id && active.generation}<SegmentSkip
        mediaId={choice.id}
        fileId={active.file_id}
        generation={active.generation}
        {position}
        {paused}
        {busy}
        {seek}
      />{/if}
    <div class="controls">
      <label
        >Volume<input
          aria-label="Playback volume"
          type="range"
          min="0"
          max="1"
          step="0.01"
          value="1"
          oninput={(e) => (player.volume = Number(e.currentTarget.value))}
        /></label
      >
      <button class="primary" disabled={busy} onclick={() => void toggle()}
        >{paused ? 'Play' : 'Pause'}</button
      ><span
        >{active.live
          ? 'Live'
          : `${time(position)} / ${time(active.duration)}`}</span
      >{#if !active.live}<label class="seek"
          >Position<input
            aria-label="Playback position"
            type="range"
            min="0"
            max={active.duration}
            step="0.1"
            value={position}
            disabled={busy}
            onchange={(e) => void seek(Number(e.currentTarget.value))}
          /></label
        >{/if}
      <button class="secondary" onclick={() => void player.requestFullscreen()}
        >Fullscreen</button
      >
    </div>
    <div class="inline-form">
      <label
        >Quality<select
          bind:value={quality}
          onchange={() => void changeOptions()}
          disabled={busy}
          ><option value="auto">Auto</option><option value="original"
            >Original</option
          >{#each [2, 4, 8, 20] as rate (rate)}<option value={`${rate}mbps`}
              >{rate} Mbps</option
            >{/each}</select
        ></label
      >
      <label
        >Audio<select
          bind:value={audio}
          onchange={() => void changeOptions()}
          disabled={busy}
          ><option value={null}>Default</option
          >{#each active.tracks.filter((t) => t.kind === 'audio') as track (track.id)}<option
              value={track.index}
              >{track.language} {track.title} ({track.codec})</option
            >{/each}</select
        ></label
      >
      <label
        >Subtitles<select bind:value={subtitle} onchange={selectSubtitle}
          ><option value="off">Off</option
          >{#each active.subtitles as track (track.id)}<option value={track.id}
              >{track.language} {track.title}</option
            >{/each}</select
        ></label
      ><span class="badge"
        >{active.mode === 'direct'
          ? 'Original file'
          : active.mode === 'remux'
            ? 'Original codecs'
            : 'Converted'}{info?.watched ? ' · Watched' : ''}</span
      >
    </div>
  {/if}
</section>

<style>
  .player {
    position: relative;
    margin: 0 0 28px;
    background: #101c21;
  }
  .player video {
    width: 100%;
    max-height: 60vh;
    background: #05090b;
    border-radius: 6px;
  }
  .player video.music {
    height: 70px;
    background: linear-gradient(90deg, #29483d, #22313d);
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 18px;
    margin: 16px 0;
  }
  .seek {
    flex: 1;
    font-size: 0;
  }
  .seek input {
    padding: 0;
    width: 100%;
  }
  .inline-form {
    align-items: end;
  }
  span {
    white-space: nowrap;
  }
  @media (max-width: 700px) {
    .controls {
      flex-wrap: wrap;
      gap: 10px;
    }
    .seek {
      min-width: 100%;
    }
  }
</style>
