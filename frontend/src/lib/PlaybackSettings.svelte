<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import type { Preferences } from './playback';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { errorClass, inlineFormClass } from './ui/styles';
  let value = $state<Preferences | null>(null),
    error = $state(''),
    saved = $state(false);
  onMount(() => {
    api<Preferences>('/playback/preferences')
      .then((v) => (value = v))
      .catch((e) => (error = String(e)));
  });
  async function save() {
    try {
      await api('/playback/preferences', 'PUT', value);
      saved = true;
    } catch (e) {
      error = String(e);
    }
  }
</script>

<Panel>
  <h2>Playback</h2>
  {#if error}<p role="alert" class={errorClass}>{error}</p>{/if}
  {#if value}<form
      class={inlineFormClass}
      onsubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <label
        >Default quality<select bind:value={value.quality}
          ><option value="auto">Auto</option><option value="original"
            >Original</option
          >{#each [2, 4, 8, 20] as rate (rate)}<option value={`${rate}mbps`}
              >{rate} Mbps</option
            >{/each}</select
        ></label
      >
      <label
        >Audio language<input
          maxlength="16"
          placeholder="eng"
          bind:value={value.audio_language}
        /></label
      >
      <label
        >Subtitle language<input
          maxlength="16"
          placeholder="eng"
          bind:value={value.subtitle_language}
        /></label
      >
      <label
        >Subtitles<select bind:value={value.subtitles}
          ><option value={false}>Off by default</option><option value={true}
            >Preferred language</option
          ></select
        ></label
      >
      <label
        >ReplayGain<select bind:value={value.replay_gain}
          ><option value="track">Track</option><option value="album"
            >Album</option
          ><option value="off">Off</option></select
        ></label
      >
      <Button type="submit" variant="secondary" size="form"
        >Save playback preferences</Button
      >{#if saved}<span role="status">Saved</span>{/if}
    </form>{/if}
</Panel>
