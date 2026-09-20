<script lang="ts">
  import Switch from './providers/components/ui/Switch.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
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
    busy = $state(false),
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
    void work(refresh);
  });
</script>

<section class="panel segment-settings">
  {#if !admin}
    <h2>Intro and credit skipping</h2>
    <p>
      Ask shows a skip button. Auto seeks past the segment while playing. Ignore
      leaves it untouched. Jellyfin clients use their own skip preferences.
    </p>
    <div class="choices">
      {#each kinds as kind (kind)}
        <div class="segment-choice">
          <span>{kind}</span>
          <ExclusiveChoiceGroup
            {choices}
            value={preferences[kind]}
            ariaLabel={`${kind} skipping`}
            disabled={busy}
            onChange={(value) => (preferences[kind] = value)}
          />
        </div>
      {/each}
    </div>
    <button
      class="primary"
      disabled={busy}
      onclick={() => work(() => api('/me/segments', 'PUT', preferences))}
      >Save skip preferences</button
    >
  {:else}<h2>Episode analysis</h2>
    <Switch bind:checked={config.local} disabled={busy}
      >Detect recurring intro and credit audio locally</Switch
    >
    <Switch bind:checked={config.external} disabled={busy}
      >Fetch TheIntroDB timestamps for confirmed episode matches</Switch
    >
    <p>
      External lookups send the show's public identifier, episode coordinates
      and runtime. Local analysis runs one task at a time while playback and
      library work are idle. Correct timestamps from an episode's library
      details.
    </p>
    <div class="actions">
      <button
        class="primary"
        disabled={busy}
        onclick={() => work(() => api('/admin/segments', 'PUT', config))}
        >Save analysis settings</button
      ><button class="secondary" disabled={busy} onclick={() => work(refresh)}
        >Refresh analysis</button
      >
    </div>
    {#each items as item, i (`${item.media_id}-${i}`)}<p>
        {item.title} · {item.state}{#if item.error}
          · {item.error}{/if}
      </p>{/each}
  {/if}
  {#if message}<p role="alert">{message}</p>{/if}
</section>

<style>
  .segment-settings {
    display: grid;
    gap: 1rem;
  }
  .choices {
    display: grid;
    justify-items: start;
    gap: 1rem;
  }
  .actions {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .segment-choice {
    display: grid;
    grid-template-columns: 4rem max-content;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.8rem;
  }
  button {
    justify-self: start;
  }
</style>
