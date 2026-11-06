<script lang="ts">
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import Switch from './ui/Switch.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import EpisodeAnalysis from './EpisodeAnalysis.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import { cardBlockClass } from './ui/styles';
  let { admin = false } = $props<{ admin?: boolean }>();
  const ids = $props.id();
  const kinds = ['Intro', 'Recap', 'Credits', 'Preview'];
  const choices = ['Ask', 'Auto', 'Ignore'].map((value) => ({
    value,
    label: value,
  }));
  let preferences = $state<Record<string, string>>({
    Intro: 'Ask',
    Recap: 'Ask',
    Credits: 'Ask',
    Preview: 'Ask',
  });
  let config = $state({ local: true, external: false }),
    busy = $state(true),
    savingConfig = $state(false),
    savingPreferences = $state(false),
    message = $state('');
  async function refresh() {
    if (admin) {
      const result = await api<{ config: typeof config }>('/admin/segments');
      config = result.config;
    } else {
      preferences = await api('/me/segments');
    }
  }
  async function work(fn: () => Promise<unknown>) {
    busy = true;
    message = '';
    try {
      await fn();
      await refresh();
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
  onMount(() => {
    void work(async () => {});
  });
</script>

{#if !admin}
  <SettingsCard
    title="Intro and credit skipping"
    description="Ask shows a skip button. Auto seeks past the segment while playing. Ignore leaves it untouched. Jellyfin clients use their own skip preferences."
    autoSave
    saving={savingPreferences}
  >
    {#if !busy && !message}<AutoSaveForm
        label="Skipping preferences"
        class="divide-y divide-line"
        bind:busy={savingPreferences}
        value={preferences}
        onRevert={(previous) => (preferences = previous)}
        onsave={(submitted) => api('/me/segments', 'PUT', submitted)}
      >
        {#snippet children(save)}
          {#each kinds as kind (kind)}
            <SettingRow label={kind}>
              <ExclusiveChoiceGroup
                {choices}
                value={preferences[kind]}
                ariaLabel={`${kind} skipping`}
                onChange={(value) => {
                  preferences[kind] = value;
                  void save();
                }}
              />
            </SettingRow>
          {/each}
        {/snippet}
      </AutoSaveForm>{:else if busy}<div class={cardBlockClass}>
        <ContentSkeleton
          label="Loading skipping preferences"
          variant="settings"
        />
      </div>{/if}
    {#if message}<p
        role="alert"
        class="m-0 {cardBlockClass} text-xs text-danger"
      >
        {message}
      </p>{/if}
  </SettingsCard>
{:else}
  <SettingsCard
    title="Detection"
    description="How intro and credit timestamps are found for episodes in the library."
    autoSave
    saving={savingConfig}
  >
    {#if !busy && !message}
      <AutoSaveForm
        label="Episode analysis settings"
        class="divide-y divide-line"
        value={config}
        bind:busy={savingConfig}
        onRevert={(previous) => (config = previous)}
        onsave={(submitted) => api('/admin/segments', 'PUT', submitted)}
      >
        <SettingRow
          label="Detect recurring intro and credit audio locally"
          description="Runs one task at a time while playback and library work are idle."
          for={`${ids}-local`}
        >
          <Switch id={`${ids}-local`} bind:checked={config.local} />
        </SettingRow>
        <SettingRow
          label="Fetch TheIntroDB timestamps for confirmed episode matches"
          description="External lookups send the show's public identifier, episode coordinates and runtime."
          for={`${ids}-external`}
        >
          <Switch id={`${ids}-external`} bind:checked={config.external} />
        </SettingRow>
      </AutoSaveForm>
    {:else if busy}
      <div class={cardBlockClass}>
        <ContentSkeleton
          label="Loading episode analysis settings"
          variant="settings"
          count={2}
        />
      </div>
    {/if}
    {#if message}<p
        role="alert"
        class="m-0 {cardBlockClass} text-xs text-danger"
      >
        {message}
      </p>{/if}
    {#snippet footer()}
      <p class="m-0 text-xs text-muted">
        Correct timestamps from an episode's library details.
      </p>
    {/snippet}
  </SettingsCard>
  <EpisodeAnalysis />
{/if}
