<script lang="ts">
  import SectionHeading from './ui/SectionHeading.svelte';
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { twMerge } from 'tailwind-merge';
  import { onDestroy, untrack } from 'svelte';
  import { GaplessQueue, StreamingRequired, type MusicState } from './gapless';
  import { time, type MediaChoice } from './playback';
  import Player from './Player.svelte';
  import { appearance, updateAppearance } from './appearance';
  import { choiceIndex } from './media-state';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';

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
  />{:else}<Panel aria-label="Music player">
    <SectionHeading>
      <h2>{musicState?.title ?? choice.title}</h2>
      <Button
        variant="secondary"
        size="form"
        onclick={async () => {
          await queue?.close();
          closed();
        }}>Close player</Button
      >
    </SectionHeading>
    {#if error}<Notice variant="error" role="alert">{error}</Notice
      >{/if}{#if busy}<p role="status">Preparing music…</p>{/if}
    {#if musicState}<div class="flex flex-wrap items-center gap-[15px]">
        <FormField class="min-w-45 flex-1"
          >Volume<input
            class={twMerge(formControlClass, 'w-full p-0')}
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
          /></FormField
        >
        <Button size="form" onclick={() => void queue?.toggle()}
          >{musicState.paused ? 'Play' : 'Pause'}</Button
        ><Button
          variant="secondary"
          size="form"
          onclick={() => void queue?.skip()}>Next track</Button
        ><span>{time(musicState.position)} / {time(musicState.duration)}</span
        ><FormField class="min-w-45 flex-1"
          >Music position<input
            class={twMerge(formControlClass, 'w-full p-0')}
            aria-label="Music position"
            type="range"
            min="0"
            max={musicState.duration}
            step="0.1"
            value={musicState.position}
            onchange={(e) => queue?.seek(Number(e.currentTarget.value))}
          /></FormField
        >
      </div>
      <p class="text-muted">
        Track {Math.min(musicState.index + 1, musicState.count)} of {musicState.count}
        · Gapless · ReplayGain {Number(
          20 * Math.log10(musicState.gain),
        ).toFixed(1)} dB
      </p>{/if}
  </Panel>{/if}
