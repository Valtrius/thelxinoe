<script lang="ts">
  import { api } from './api';
  import { serverUpdates } from './server-updates';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import { cardBlockClass, formControlClass } from './ui/styles';
  let policy = $state('automatic'),
    start = $state(3),
    end = $state(5);
  let initialized = $state(false);
  $effect(() => {
    if ($serverUpdates.status && !initialized) {
      const saved = $serverUpdates.status.policy;
      policy = saved.policy;
      start = saved.window_start;
      end = saved.window_end;
      initialized = true;
    }
  });
</script>

{#if initialized}
  <AutoSaveForm
    label="Server update settings"
    class="divide-y divide-line"
    value={{ policy, window_start: start, window_end: end }}
    onRevert={(previous) => {
      policy = previous.policy;
      start = previous.window_start;
      end = previous.window_end;
    }}
    onsave={(submitted) =>
      api('/admin/product-update/policy', 'POST', submitted)}
  >
    {#snippet children(save)}
      <SettingRow
        label="Server updates"
        description="Notify shows a badge when a release is ready. Automatic installs it in the maintenance window."
      >
        <ExclusiveChoiceGroup
          ariaLabel="Update policy"
          value={policy}
          choices={[
            { value: 'notify', label: 'Notify' },
            { value: 'automatic', label: 'Automatic' },
          ]}
          onChange={(value) => {
            policy = value;
            void save();
          }}
        />
      </SettingRow>
      <!-- Automatic backups use this window even when updates only notify. -->
      <SettingRow
        label="Maintenance window"
        description={`Automatic updates and backups run between these hours, ${$serverUpdates.status?.timezone ?? 'UTC'}.${start === end ? ' The same start and end keep it open all day.' : ''}`}
      >
        <input
          aria-label="Maintenance starts"
          class={[formControlClass, 'h-8.5! w-16! px-2! py-1! text-center']}
          type="number"
          min="0"
          max="23"
          required
          bind:value={start}
        />
        <span class="text-xs text-muted">to</span>
        <input
          aria-label="Maintenance ends"
          class={[formControlClass, 'h-8.5! w-16! px-2! py-1! text-center']}
          type="number"
          min="0"
          max="23"
          required
          bind:value={end}
        />
      </SettingRow>
    {/snippet}
  </AutoSaveForm>
{:else}
  <div class={cardBlockClass}>
    <ContentSkeleton
      label="Loading update settings"
      variant="settings"
      count={2}
    />
  </div>
{/if}
