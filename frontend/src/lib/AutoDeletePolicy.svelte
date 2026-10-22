<script lang="ts" module>
  export type Policy = {
    domain: 'movies' | 'shows' | 'videos';
    enabled: boolean;
    grace_seconds: number;
    exclude_specials: boolean;
    trigger_users: string[];
    storage_limit_bytes: number;
  };
  export type VideoUsage = {
    unpinned_bytes: number;
    pinned_bytes: number;
    waiting: number;
  };
</script>

<script lang="ts">
  import { Search } from '@lucide/svelte';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import Switch from './ui/Switch.svelte';
  import { formControlClass } from './ui/styles';
  import { twMerge } from 'tailwind-merge';

  let {
    policy = $bindable(),
    users,
    usage,
    onSave,
  }: {
    policy: Policy;
    users: { id: string; username: string }[];
    usage: VideoUsage;
    onSave: (value: Policy) => Promise<unknown>;
  } = $props();
  const ids = $props.id();
  let saving = $state(false);
  let filter = $state('');
  const video = $derived(policy.domain === 'videos');
  const title = $derived(
    video
      ? 'Video downloads'
      : policy.domain === 'movies'
        ? 'Movies'
        : 'TV seasons',
  );
  const description = $derived(
    video
      ? 'YouTube, Twitch and Kick videos saved to this server.'
      : policy.domain === 'movies'
        ? 'Radarr-tracked files only. Files without a manager stay.'
        : 'Sonarr-tracked files in complete seasons only. Files without a manager stay.',
  );
  // Long user lists stay usable: a bounded, searchable list instead of chips.
  const searchable = $derived(users.length > 6);
  const shown = $derived(
    users.filter((user) =>
      user.username.toLowerCase().includes(filter.trim().toLowerCase()),
    ),
  );
  const gb = (bytes: number) =>
    (bytes / 1e9).toLocaleString(undefined, { maximumFractionDigits: 1 });
  const limit = $derived((policy.storage_limit_bytes ?? 100e9) / 1e9);
  const share = (bytes: number) =>
    Math.max(0, Math.min(100, (bytes / 1e9 / Math.max(limit, 0.001)) * 100));
  function setWatchers(next: string[]) {
    policy.trigger_users = next;
    if (!next.length) policy.enabled = false;
  }
</script>

<SettingsCard
  role="group"
  aria-label={title}
  {title}
  {description}
  autoSave
  {saving}
>
  <AutoSaveForm
    value={policy}
    onsave={onSave}
    bind:busy={saving}
    onRevert={(value) => (policy = value)}
    label={`${title} auto-delete settings`}
    class="divide-y divide-line"
  >
    {#snippet children(save)}
      {#if video}
        <SettingRow
          label="Delete downloads"
          description="Storage limit removes the oldest downloads when the limit is reached. After watching also removes a download once everyone who saved or requested it has watched."
          for={`${ids}-mode`}
          control="field"
        >
          <select
            id={`${ids}-mode`}
            aria-label="Delete downloads"
            class={formControlClass}
            value={policy.enabled ? 'after-watching' : 'storage-limit'}
            onchange={(event) =>
              (policy.enabled = event.currentTarget.value === 'after-watching')}
          >
            <option value="storage-limit">Storage limit</option>
            <option value="after-watching">After watching</option>
          </select>
        </SettingRow>
      {:else}
        <SettingRow
          label={`Auto-delete watched ${policy.domain === 'movies' ? 'movies' : 'seasons'}`}
          description={policy.trigger_users.length
            ? 'Delete after everyone selected below has watched.'
            : 'Choose at least one person below to turn this on.'}
          for={`${ids}-enabled`}
        >
          <Switch
            id={`${ids}-enabled`}
            bind:checked={policy.enabled}
            disabled={!policy.trigger_users.length}
          />
        </SettingRow>
        <SettingRow
          label="Watched by"
          description={`${policy.trigger_users.length} of ${users.length} selected.`}
          control="field"
        >
          <div
            class="w-full min-w-0 border border-line bg-surface-strong text-xs"
            role="group"
            aria-label="People who must have watched"
          >
            {#if searchable}
              <label class="relative block border-b border-line">
                <span class="sr-only">Find a person</span>
                <Search
                  size={13}
                  class="pointer-events-none absolute top-2.5 left-2.5 text-muted"
                  aria-hidden="true"
                />
                <input
                  type="search"
                  class="w-full border-0 bg-transparent py-2 pr-2 pl-8 text-xs text-foreground placeholder:text-muted focus:outline-none"
                  placeholder="Find a person"
                  bind:value={filter}
                />
              </label>
            {/if}
            <div class="max-h-44 overflow-y-auto">
              {#each shown as user (user.id)}
                <label
                  class="flex cursor-pointer items-center gap-2.5 px-3 py-1.75 hover:bg-surface-soft has-checked:bg-accent-soft"
                >
                  <input
                    type="checkbox"
                    class="size-3.5 shrink-0 accent-accent"
                    checked={policy.trigger_users.includes(user.id)}
                    onchange={(event) =>
                      setWatchers(
                        event.currentTarget.checked
                          ? [...policy.trigger_users, user.id]
                          : policy.trigger_users.filter((id) => id !== user.id),
                      )}
                  />
                  <span class="min-w-0 truncate">{user.username}</span>
                </label>
              {:else}
                <p class="m-0 px-3 py-2 text-muted">
                  {users.length ? 'No one matches.' : 'No people yet.'}
                </p>
              {/each}
            </div>
            {#if users.length > 1}
              <div
                class="flex items-center justify-end gap-3 border-t border-line px-3 py-1.5 text-[11px]"
              >
                <button
                  type="button"
                  class="text-accent hover:underline"
                  disabled={policy.trigger_users.length === users.length}
                  onclick={() => {
                    setWatchers(users.map((user) => user.id));
                    void save();
                  }}>Select everyone</button
                >
                <button
                  type="button"
                  class="text-muted hover:underline"
                  disabled={!policy.trigger_users.length}
                  onclick={() => {
                    setWatchers([]);
                    void save();
                  }}>Clear</button
                >
              </div>
            {/if}
          </div>
        </SettingRow>
      {/if}
      {#if policy.enabled}
        <SettingRow
          label="Delete after (days)"
          description={video
            ? 'After everyone saving or requesting the download has watched. At least 1 day.'
            : 'Grace period after the last selected person has watched. At least 1 day.'}
          for={`${ids}-grace`}
        >
          <input
            id={`${ids}-grace`}
            class={twMerge(formControlClass, 'w-24')}
            type="number"
            min="1"
            max="365"
            step="any"
            required
            value={policy.grace_seconds / 86400}
            onchange={(event) => {
              if (!event.currentTarget.validity.valid) return;
              policy.grace_seconds = Math.round(
                Number(event.currentTarget.value) * 86400,
              );
            }}
          />
        </SettingRow>
        {#if policy.domain === 'shows'}
          <SettingRow
            label="Exclude specials (Season 0)"
            for={`${ids}-specials`}
          >
            <Switch
              id={`${ids}-specials`}
              bind:checked={policy.exclude_specials}
            />
          </SettingRow>
        {/if}
      {/if}
      {#if video}
        <SettingRow
          label="Unpinned storage limit (GB)"
          description="Oldest downloads go first, even unwatched. Pins stay outside the limit."
          for={`${ids}-limit`}
        >
          <div class="grid justify-items-end gap-2 compact:justify-items-start">
            <input
              id={`${ids}-limit`}
              class={twMerge(formControlClass, 'w-32')}
              type="number"
              min="40"
              max="9000000"
              step="any"
              required
              value={limit}
              onchange={(event) => {
                if (!event.currentTarget.validity.valid) return;
                policy.storage_limit_bytes = Math.round(
                  Number(event.currentTarget.value) * 1e9,
                );
              }}
            />
            <div
              class="relative h-1.5 w-72 max-w-full bg-line"
              aria-hidden="true"
            >
              <span
                class="absolute inset-y-0 left-0 bg-accent"
                style:width={`${share(usage.unpinned_bytes)}%`}
              ></span>
              <span
                class="absolute inset-y-0 bg-warning"
                style:left={`${share(usage.unpinned_bytes)}%`}
                style:width={`${Math.min(share(usage.pinned_bytes), 100 - share(usage.unpinned_bytes))}%`}
              ></span>
            </div>
            <p class="m-0 text-right text-xs text-muted compact:text-left">
              {gb(usage.unpinned_bytes)} GB unpinned · {gb(usage.pinned_bytes)} GB
              pinned
            </p>
            {#if usage.waiting > 0}<p
                class="m-0 text-right text-xs text-foreground compact:text-left"
                role="status"
              >
                {usage.waiting}
                {usage.waiting === 1 ? 'download waiting' : 'downloads waiting'} for
                space.
              </p>{/if}
          </div>
        </SettingRow>
      {/if}
    {/snippet}
  </AutoSaveForm>
</SettingsCard>
