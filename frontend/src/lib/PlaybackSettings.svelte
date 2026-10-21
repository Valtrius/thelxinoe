<script lang="ts">
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import Notice from './ui/Notice.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import { formControlClass } from './ui/styles';
  import { onMount } from 'svelte';
  import { api } from './api';
  import type { Preferences } from './playback';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  const ids = $props.id();
  let value = $state<Preferences | null>(null),
    error = $state(''),
    saving = $state(false);
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

<div class="col-span-full grid min-w-0 gap-5">
  {#if error}<Notice role="alert" variant="error" class="mb-0">{error}</Notice
    >{/if}
  {#if value}<AutoSaveForm
      label="Playback preferences"
      class="grid gap-5"
      bind:busy={saving}
      {value}
      onRevert={(previous) => (value = previous)}
      onsave={(submitted) => api('/playback/preferences', 'PUT', submitted)}
    >
      <SettingsCard title="Video and audio" autoSave {saving}>
        <SettingRow
          label="Default quality"
          description="Auto adapts to your connection. Original never transcodes."
          for={`${ids}-quality`}
          control="field"
        >
          <select
            id={`${ids}-quality`}
            class={formControlClass}
            bind:value={value.quality}
            ><option value="auto">Auto</option><option value="original"
              >Original</option
            >{#each [4320, 2160, 1440, 1080, 720, 480, 360] as height (height)}<option
                value={`${height}p`}>{height}p</option
              >{/each}</select
          >
        </SettingRow>
        <SettingRow
          label="Audio language"
          description="Preferred audio track as a language code, for example eng."
          for={`${ids}-audio`}
          control="field"
        >
          <input
            id={`${ids}-audio`}
            class={formControlClass}
            maxlength="16"
            placeholder="eng"
            bind:value={value.audio_language}
          />
        </SettingRow>
        <SettingRow
          label="ReplayGain"
          description="Evens out loudness across tracks or whole albums."
          for={`${ids}-gain`}
          control="field"
        >
          <select
            id={`${ids}-gain`}
            class={formControlClass}
            bind:value={value.replay_gain}
            ><option value="track">Track</option><option value="album"
              >Album</option
            ><option value="off">Off</option></select
          >
        </SettingRow>
      </SettingsCard>
      <SettingsCard title="Subtitles" autoSave {saving}>
        <SettingRow
          label="Subtitle language"
          description="Preferred subtitle track as a language code."
          for={`${ids}-subtitle-language`}
          control="field"
        >
          <input
            id={`${ids}-subtitle-language`}
            class={formControlClass}
            maxlength="16"
            placeholder="eng"
            bind:value={value.subtitle_language}
          />
        </SettingRow>
        <SettingRow
          label="Subtitles"
          description="Whether subtitles start enabled in your preferred language."
          for={`${ids}-subtitles`}
          control="field"
        >
          <select
            id={`${ids}-subtitles`}
            class={formControlClass}
            bind:value={value.subtitles}
            ><option value={false}>Off by default</option><option value={true}
              >Preferred language</option
            ></select
          >
        </SettingRow>
      </SettingsCard>
    </AutoSaveForm>{:else if !error}<ContentSkeleton
      label="Loading playback preferences"
      variant="settings"
    />{/if}
</div>
