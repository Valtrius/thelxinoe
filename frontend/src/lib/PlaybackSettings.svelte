<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { onMount } from 'svelte';
  import { api } from './api';
  import type { Preferences } from './playback';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import Panel from './ui/Panel.svelte';
  import { inlineFormClass } from './ui/styles';
  let value = $state<Preferences | null>(null),
    error = $state('');
  onMount(() => {
    api<Preferences>('/playback/preferences')
      .then((v) => (value = v))
      .catch((e) => (error = String(e)));
  });
</script>

<Panel>
  <h2>Playback</h2>
  {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
  {#if value}<AutoSaveForm
      label="Playback preferences"
      class={inlineFormClass}
      {value}
      onRevert={(previous) => (value = previous)}
      onsave={(submitted) => api('/playback/preferences', 'PUT', submitted)}
    >
      <FormField
        >Default quality<select
          class={formControlClass}
          bind:value={value.quality}
          ><option value="auto">Auto</option><option value="original"
            >Original</option
          >{#each [2, 4, 8, 20] as rate (rate)}<option value={`${rate}mbps`}
              >{rate} Mbps</option
            >{/each}</select
        ></FormField
      >
      <FormField
        >Audio language<input
          class={formControlClass}
          maxlength="16"
          placeholder="eng"
          bind:value={value.audio_language}
        /></FormField
      >
      <FormField
        >Subtitle language<input
          class={formControlClass}
          maxlength="16"
          placeholder="eng"
          bind:value={value.subtitle_language}
        /></FormField
      >
      <FormField
        >Subtitles<select class={formControlClass} bind:value={value.subtitles}
          ><option value={false}>Off by default</option><option value={true}
            >Preferred language</option
          ></select
        ></FormField
      >
      <FormField
        >ReplayGain<select
          class={formControlClass}
          bind:value={value.replay_gain}
          ><option value="track">Track</option><option value="album"
            >Album</option
          ><option value="off">Off</option></select
        ></FormField
      >
    </AutoSaveForm>{/if}
</Panel>
