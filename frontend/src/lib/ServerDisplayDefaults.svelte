<script lang="ts">
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import { onDestroy, untrack } from 'svelte';
  import { api } from './api';
  import { LatestRequest } from './latest-request';
  import { captureSession } from './session';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import Notice from './ui/Notice.svelte';
  import Button from './ui/Button.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import TimezoneSelect from './TimezoneSelect.svelte';
  import { cardBlockClass, formControlClass } from './ui/styles';

  let { revision = 0 } = $props<{
    revision?: number;
  }>();
  type Defaults = { timezone: string; time_format: '12h' | '24h' };
  const ids = $props.id();
  let confirmed = $state<Defaults | null>(null);
  let draft = $state<Defaults>({ timezone: 'UTC', time_format: '24h' });
  let error = $state(''),
    saving = $state(false),
    refreshPending = $state(false);
  const dirty = $derived(
    confirmed && JSON.stringify(draft) !== JSON.stringify(confirmed),
  );
  const requests = new LatestRequest();
  const ownsSession = captureSession();
  let active = true;
  onDestroy(() => {
    active = false;
    requests.invalidate();
  });
  $effect(() => {
    void revision;
    untrack(() => void load());
  });
  $effect(() => {
    if (!saving && !dirty && refreshPending) untrack(() => void load());
  });
  async function load() {
    refreshPending = saving || Boolean(dirty);
    const current = requests.begin();
    const previous = JSON.stringify(draft);
    error = '';
    try {
      const value = await api<Defaults>('/admin/settings');
      if (!current()) return;
      if (
        confirmed &&
        (saving ||
          JSON.stringify(draft) !== previous ||
          previous !== JSON.stringify(confirmed))
      ) {
        refreshPending = true;
        return;
      }
      refreshPending = false;
      confirmed = { timezone: value.timezone, time_format: value.time_format };
      draft = { ...confirmed };
    } catch (caught) {
      if (current()) error = String(caught);
    }
  }
  async function save(submitted: Defaults) {
    if (!active || !ownsSession()) return;
    requests.invalidate();
    await api('/admin/settings', 'PUT', submitted);
    if (!active || !ownsSession()) return;
    requests.invalidate();
    confirmed = { ...submitted };
  }
</script>

<SettingsCard
  id="server-display"
  title="Display defaults"
  description="Used by everyone who has not chosen their own timezone or time format."
  autoSave
  {saving}
>
  {#if confirmed}
    <AutoSaveForm
      label="Server display defaults"
      class="divide-y divide-line"
      value={draft}
      baseline={confirmed}
      onsave={save}
      bind:busy={saving}
      onRevert={(previous) => (draft = previous)}
    >
      <SettingRow
        label="Server default timezone"
        for={`${ids}-timezone`}
        control="field"
      >
        <TimezoneSelect
          id={`${ids}-timezone`}
          label="Server default timezone"
          bind:value={draft.timezone}
        />
      </SettingRow>
      <SettingRow
        label="Server default time format"
        for={`${ids}-format`}
        control="field"
      >
        <select
          id={`${ids}-format`}
          class={formControlClass}
          bind:value={draft.time_format}
        >
          <option value="24h">24-hour</option><option value="12h"
            >12-hour</option
          >
        </select>
      </SettingRow>
    </AutoSaveForm>
  {:else if !error}
    <div class={cardBlockClass}>
      <ContentSkeleton
        label="Loading display defaults"
        variant="settings"
        count={2}
      />
    </div>
  {/if}
  {#if error}
    <div class="{cardBlockClass} grid justify-items-start gap-3">
      <Notice role="alert" variant="error" class="mb-0">{error}</Notice>
      <Button variant="secondary" size="form" onclick={() => void load()}
        >Retry display defaults</Button
      >
    </div>
  {/if}
</SettingsCard>
