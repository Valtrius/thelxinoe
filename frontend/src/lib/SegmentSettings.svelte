<script lang="ts">
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import Switch from './ui/Switch.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import { cardBlockClass, cardTableClass } from './ui/styles';
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
    items = $state<
      { media_id: string; title: string; state: string; error: string | null }[]
    >([]),
    busy = $state(true),
    savingConfig = $state(false),
    savingPreferences = $state(false),
    message = $state('');
  const pending = $derived(
    items.filter((item) => !['complete', 'failed'].includes(item.state)).length,
  );
  const tone = (state: string) =>
    state === 'complete'
      ? 'ok'
      : state === 'failed'
        ? 'bad'
        : state === 'pending' || state === 'queued'
          ? 'warn'
          : 'info';
  async function refresh() {
    if (admin) {
      const result = await api<{ config: typeof config; items: typeof items }>(
        '/admin/segments',
      );
      config = result.config;
      items = result.items;
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
    {#snippet actions()}
      <Button
        variant="secondary"
        size="form"
        disabled={busy || savingConfig}
        onclick={() => work(async () => {})}>Refresh analysis</Button
      >
    {/snippet}
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
  <SettingsCard title="Analysis queue">
    {#snippet actions()}
      <span class="text-xs text-muted"
        >{items.length}
        {items.length === 1 ? 'episode' : 'episodes'}{pending
          ? ` · ${pending} pending`
          : ''}</span
      >
    {/snippet}
    {#if items.length}
      <table class={cardTableClass}>
        <thead><tr><th>Episode</th><th class="w-32">State</th></tr></thead>
        <tbody>
          {#each items as item, i (`${item.media_id}-${i}`)}
            <tr>
              <td
                >{item.title}{#if item.error}<small class="mt-1 text-danger"
                    >{item.error}</small
                  >{/if}</td
              >
              <td
                ><StatusChip tone={tone(item.state)}>{item.state}</StatusChip
                ></td
              >
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="m-0 {cardBlockClass} text-xs text-muted">
        No episodes queued for analysis.
      </p>
    {/if}
  </SettingsCard>
{/if}
