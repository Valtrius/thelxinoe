<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { GaplessQueue, StreamingRequired, type MusicState } from './gapless';
  import { time, type MediaChoice } from './playback';
  import Player from './Player.svelte';
  import { appearance, updateAppearance } from './appearance';
  import { choiceIndex } from './media-state';
  let { choice, closed } = $props<{
    choice: MediaChoice;
    closed: () => void;
  }>();
  let musicState = $state<MusicState | null>(null),
    error = $state(''),
    busy = $state(true),
    streaming = $state(false);
  let streamChoice = $state<MediaChoice | null>(null);
  let queue = $state<GaplessQueue>();
  let generation = 0;
  $effect(() => {
    queue?.setVolume($appearance.audio_volume);
  });
  $effect(() => {
    const selected = choice;
    untrack(() => void open(selected));
  });
  async function open(selected: MediaChoice) {
    const revision = ++generation;
    await queue?.close();
    musicState = null;
    busy = true;
    error = '';
    streaming = false;
    const choices = selected.queue ?? [selected];
    const index = choiceIndex(choices, selected);
    queue = new GaplessQueue(
      [
        { ...choices[index], ...selected, queue: undefined },
        ...choices.slice(index + 1),
      ],
      (value) => {
        if (generation === revision) musicState = value;
      },
      (e) => {
        if (generation === revision) {
          if (e instanceof StreamingRequired) {
            streamChoice = e.choice ?? selected;
            streaming = true;
          } else error = String(e);
        }
      },
    );
    try {
      queue.setVolume($appearance.audio_volume);
      await queue.start();
    } catch (e) {
      await queue.close();
      if (e instanceof StreamingRequired) {
        streamChoice = e.choice ?? selected;
        streaming = true;
      } else error = String(e);
    } finally {
      if (generation === revision) busy = false;
    }
  }
  onDestroy(() => {
    generation++;
    void queue?.close();
  });
  function nextStream() {
    const choices: MediaChoice[] = choice.queue ?? [choice];
    const next = choices[choiceIndex(choices, streamChoice!) + 1];
    if (next) streamChoice = next;
  }
</script>

{#if streaming && streamChoice}<Player
    choice={streamChoice}
    {closed}
    ended={nextStream}
  />{:else}<section class="panel" aria-label="Music player">
    <div class="section-heading">
      <h2>{musicState?.title ?? choice.title}</h2>
      <button
        class="secondary"
        onclick={async () => {
          await queue?.close();
          closed();
        }}>Close player</button
      >
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}{#if busy}<p
        role="status"
      >
        Preparing music…
      </p>{/if}
    {#if musicState}<div class="controls">
        <label
          >Volume<input
            aria-label="Music volume"
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={$appearance.audio_volume}
            oninput={(e) => {
              const volume = Number(e.currentTarget.value);
              queue?.setVolume(volume);
              updateAppearance({ audio_volume: volume });
            }}
          /></label
        >
        <button class="primary" onclick={() => void queue?.toggle()}
          >{musicState.paused ? 'Play' : 'Pause'}</button
        ><button class="secondary" onclick={() => void queue?.skip()}
          >Next track</button
        ><span>{time(musicState.position)} / {time(musicState.duration)}</span
        ><label
          >Music position<input
            aria-label="Music position"
            type="range"
            min="0"
            max={musicState.duration}
            step="0.1"
            value={musicState.position}
            onchange={(e) => queue?.seek(Number(e.currentTarget.value))}
          /></label
        >
      </div>
      <p class="muted">
        Track {Math.min(musicState.index + 1, musicState.count)} of {musicState.count}
        · Gapless · ReplayGain {Number(
          20 * Math.log10(musicState.gain),
        ).toFixed(1)} dB
      </p>{/if}
  </section>{/if}

<style>
  .controls {
    display: flex;
    align-items: center;
    gap: 15px;
    flex-wrap: wrap;
  }
  .controls label {
    flex: 1;
    min-width: 180px;
  }
  .controls input {
    width: 100%;
    padding: 0;
  }
</style>
