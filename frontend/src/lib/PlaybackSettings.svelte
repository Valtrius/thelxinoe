<script lang="ts">
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { onMount } from 'svelte';
  import { api } from './api';
  import type { Preferences } from './playback';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  let value = $state<Preferences | null>(null),
    error = $state('');
  onMount(() => {
    api<Preferences>('/playback/preferences')
      .then(
        (v) =>
          (value = {
            ...v,
            quality: v.quality.endsWith('mbps') ? 'auto' : v.quality,
          }),
      )
      .catch((e) => (error = String(e)));
  });
</script>

<div class="col-span-full">
  {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
  {#if value}<AutoSaveForm
      label="Playback preferences"
      class="settings-grid"
      {value}
      onRevert={(previous) => (value = previous)}
      onsave={(submitted) => api('/playback/preferences', 'PUT', submitted)}
    >
      <section class="settings-section">
        <h2>Video</h2>
        <FormField
          >Default quality<select
            class={formControlClass}
            bind:value={value.quality}
            ><option value="auto">Auto</option><option value="original"
              >Original</option
            >{#each [360, 480, 720, 1080, 1440, 2160] as height (height)}<option
                value={`${height}p`}>{height}p</option
              >{/each}</select
          ></FormField
        >
      </section>
      <section class="settings-section">
        <h2>Audio</h2>
        <FormField
          >Audio language<input
            class={formControlClass}
            maxlength="16"
            placeholder="eng"
            bind:value={value.audio_language}
          /></FormField
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
      </section>
      <section class="settings-section">
        <h2>Subtitles</h2>
        <FormField
          >Subtitle language<input
            class={formControlClass}
            maxlength="16"
            placeholder="eng"
            bind:value={value.subtitle_language}
          /></FormField
        >
        <FormField
          >Subtitles<select
            class={formControlClass}
            bind:value={value.subtitles}
            ><option value={false}>Off by default</option><option value={true}
              >Preferred language</option
            ></select
          ></FormField
        >
      </section>
    </AutoSaveForm>{:else if !error}<ContentSkeleton
      label="Loading playback preferences"
      variant="settings"
    />{/if}
</div>
