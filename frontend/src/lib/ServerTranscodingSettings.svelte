<script lang="ts">
  import { revisions } from './server-events.svelte';
  import { onDestroy, untrack } from 'svelte';
  import { api } from './api';
  import { LatestRequest } from './latest-request';
  import { captureSession } from './session';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import Switch from './ui/Switch.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import Notice from './ui/Notice.svelte';
  import Button from './ui/Button.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import { cardBlockClass } from './ui/styles';

  type Policy = { video_transcoding: boolean; audio_transcoding: boolean };
  const ids = $props.id();
  let confirmed = $state<Policy | null>(null);
  let draft = $state<Policy>({
    video_transcoding: true,
    audio_transcoding: true,
  });
  let saving = $state(false),
    refreshPending = $state(false),
    error = $state('');
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
    void revisions.preferences;
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
      const value = await api<Policy>('/admin/settings');
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
      confirmed = {
        video_transcoding: value.video_transcoding,
        audio_transcoding: value.audio_transcoding,
      };
      draft = { ...confirmed };
    } catch (caught) {
      if (current()) error = String(caught);
    }
  }
  async function save(submitted: Policy) {
    if (!active || !ownsSession()) return;
    requests.invalidate();
    await api<Policy>('/admin/settings', 'PUT', submitted);
    if (!active || !ownsSession()) return;
    requests.invalidate();
    confirmed = { ...submitted };
  }
</script>

<SettingsCard
  id="server-transcoding"
  title="Transcoding"
  description="Convert media on the fly when a device cannot play the original."
  autoSave
  {saving}
>
  {#if confirmed}
    <AutoSaveForm
      label="Server transcoding settings"
      class="divide-y divide-line"
      value={draft}
      baseline={confirmed}
      onsave={save}
      bind:busy={saving}
      onRevert={(previous) => (draft = previous)}
    >
      <SettingRow label="Allow video transcoding" for={`${ids}-video`}>
        <Switch id={`${ids}-video`} bind:checked={draft.video_transcoding} />
      </SettingRow>
      <SettingRow label="Allow audio transcoding" for={`${ids}-audio`}>
        <Switch id={`${ids}-audio`} bind:checked={draft.audio_transcoding} />
      </SettingRow>
    </AutoSaveForm>
  {:else if !error}
    <div class={cardBlockClass}>
      <ContentSkeleton
        label="Loading transcoding settings"
        variant="settings"
        count={2}
      />
    </div>
  {/if}
  {#if error}
    <div class="{cardBlockClass} grid justify-items-start gap-3">
      <Notice role="alert" variant="error" class="mb-0">{error}</Notice>
      <Button variant="secondary" size="form" onclick={() => void load()}
        >Retry transcoding settings</Button
      >
    </div>
  {/if}
</SettingsCard>
