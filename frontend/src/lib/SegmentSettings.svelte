<script lang="ts">
  import Switch from './ui/Switch.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  let { admin = false } = $props<{ admin?: boolean }>();
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
    message = $state('');
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
  <Panel class="grid gap-4">
    <h2>Intro and credit skipping</h2>
    <p>
      Ask shows a skip button. Auto seeks past the segment while playing. Ignore
      leaves it untouched. Jellyfin clients use their own skip preferences.
    </p>
    {#if !busy && !message}<AutoSaveForm
        label="Skipping preferences"
        class="grid justify-items-start gap-4"
        value={preferences}
        onRevert={(previous) => (preferences = previous)}
        onsave={(submitted) => api('/me/segments', 'PUT', submitted)}
      >
        {#snippet children(save)}
          {#each kinds as kind (kind)}
            <div
              class="grid grid-cols-[4rem_max-content] items-center gap-2 text-[0.8rem]"
            >
              <span>{kind}</span>
              <ExclusiveChoiceGroup
                {choices}
                value={preferences[kind]}
                ariaLabel={`${kind} skipping`}
                onChange={(value) => {
                  preferences[kind] = value;
                  void save();
                }}
              />
            </div>
          {/each}
        {/snippet}
      </AutoSaveForm>{/if}
    {#if message}<p role="alert">{message}</p>{/if}
  </Panel>
{:else}<Panel class="grid gap-4"
    ><h2>Episode analysis</h2>
    {#if !busy && !message}
      <AutoSaveForm
        label="Episode analysis settings"
        class="grid justify-items-start gap-4"
        value={config}
        bind:busy={savingConfig}
        onRevert={(previous) => (config = previous)}
        onsave={(submitted) => api('/admin/segments', 'PUT', submitted)}
      >
        <Switch bind:checked={config.local}
          >Detect recurring intro and credit audio locally</Switch
        >
        <Switch bind:checked={config.external}
          >Fetch TheIntroDB timestamps for confirmed episode matches</Switch
        >
      </AutoSaveForm>
    {/if}
    <p>
      External lookups send the show's public identifier, episode coordinates
      and runtime. Local analysis runs one task at a time while playback and
      library work are idle. Correct timestamps from an episode's library
      details.
    </p>
    <div class="flex flex-wrap gap-4">
      <Button
        variant="secondary"
        size="form"
        disabled={busy || savingConfig}
        onclick={() => work(async () => {})}>Refresh analysis</Button
      >
    </div>
    {#if message}<p role="alert">{message}</p>{/if}
  </Panel><Panel class="settings-wide"
    ><h2>Analysis queue</h2>
    {#each items as item, i (`${item.media_id}-${i}`)}<p>
        {item.title} · {item.state}{#if item.error}
          · {item.error}{/if}
      </p>{:else}<p class="text-muted">
        No episodes queued for analysis.
      </p>{/each}
  </Panel>{/if}
