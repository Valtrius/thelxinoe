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
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import FormField from './ui/FormField.svelte';
  import Panel from './ui/Panel.svelte';
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
  const video = $derived(policy.domain === 'videos');
  const title = $derived(
    video
      ? 'Video downloads'
      : policy.domain === 'movies'
        ? 'Movies'
        : 'TV seasons',
  );
  const gb = (bytes: number) =>
    (bytes / 1e9).toLocaleString(undefined, { maximumFractionDigits: 1 });
</script>

<Panel>
  <AutoSaveForm
    value={policy}
    onsave={onSave}
    onRevert={(value) => (policy = value)}
    label={`${title} auto-delete settings`}
  >
    <fieldset class="grid min-w-0 gap-4">
      <legend class="mb-4 text-base font-semibold">{title}</legend>
      {#if video}
        <FormField class="m-0"
          >Delete downloads
          <select
            aria-label="Delete downloads"
            class={formControlClass}
            value={policy.enabled ? 'after-watching' : 'storage-limit'}
            onchange={(event) =>
              (policy.enabled = event.currentTarget.value === 'after-watching')}
          >
            <option value="storage-limit">Storage limit</option>
            <option value="after-watching">After watching</option>
          </select>
        </FormField>
      {:else}
        <Switch
          bind:checked={policy.enabled}
          disabled={!policy.trigger_users.length}
          >Auto-delete watched {policy.domain === 'movies'
            ? 'movies'
            : 'seasons'}</Switch
        >
        <fieldset class="grid gap-2">
          <legend class="mb-2 text-xs text-muted"
            >After all selected people have watched</legend
          >
          <div class="flex flex-wrap gap-x-5 gap-y-2">
            {#each users as user (user.id)}
              <label class="inline-flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  class="size-4 accent-accent"
                  checked={policy.trigger_users.includes(user.id)}
                  onchange={(event) => {
                    policy.trigger_users = event.currentTarget.checked
                      ? [...policy.trigger_users, user.id]
                      : policy.trigger_users.filter((id) => id !== user.id);
                    if (!policy.trigger_users.length) policy.enabled = false;
                  }}
                />{user.username}
              </label>
            {/each}
          </div>
        </fieldset>
      {/if}
      {#if policy.enabled}
        <FormField class="m-0"
          >Delete after (days)
          <input
            class={twMerge(formControlClass, 'w-28')}
            type="number"
            min="0"
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
        </FormField>
        {#if video}
          <p class="m-0 text-xs text-muted">
            After everyone saving or requesting the download has watched.
          </p>
        {:else}
          {#if policy.domain === 'shows'}
            <Switch bind:checked={policy.exclude_specials}
              >Exclude specials (Season 0)</Switch
            >
          {/if}
        {/if}
      {/if}
      {#if video}
        <FormField class="m-0"
          >Unpinned storage limit (GB)
          <input
            class={twMerge(formControlClass, 'w-36')}
            type="number"
            min="0.001"
            max="9000000"
            step="any"
            required
            value={(policy.storage_limit_bytes ?? 100e9) / 1e9}
            onchange={(event) => {
              if (!event.currentTarget.validity.valid) return;
              policy.storage_limit_bytes = Math.round(
                Number(event.currentTarget.value) * 1e9,
              );
            }}
          />
        </FormField>
        <div class="grid gap-1 text-xs text-muted">
          <p class="m-0">
            {gb(usage.unpinned_bytes)} GB unpinned · {gb(usage.pinned_bytes)} GB pinned
          </p>
          <p class="m-0">
            Oldest downloads go first, even unwatched. Pins stay outside the
            limit.
          </p>
          {#if usage.waiting > 0}<p class="m-0 text-foreground" role="status">
              {usage.waiting}
              {usage.waiting === 1 ? 'download waiting' : 'downloads waiting'} for
              space.
            </p>{/if}
        </div>
      {:else}
        <p class="m-0 text-xs text-muted">
          {policy.domain === 'movies'
            ? 'Radarr-tracked files only.'
            : 'Sonarr-tracked files in complete seasons only.'} Files without a manager
          stay.
        </p>
      {/if}
    </fieldset>
  </AutoSaveForm>
</Panel>
