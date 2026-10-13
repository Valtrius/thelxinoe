<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { api } from './api';
  import { LatestRequest } from './latest-request';
  import { captureSession } from './session';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import Switch from './ui/Switch.svelte';
  import Panel from './ui/Panel.svelte';
  import Notice from './ui/Notice.svelte';
  import Button from './ui/Button.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';

  let { revision = 0 } = $props<{ revision?: number }>();
  type Policy = { video_transcoding: boolean; audio_transcoding: boolean };
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

<Panel>
  <h2>Transcoding</h2>
  {#if confirmed}
    <AutoSaveForm
      label="Server transcoding settings"
      class="grid justify-items-start gap-4"
      value={draft}
      baseline={confirmed}
      onsave={save}
      bind:busy={saving}
      onRevert={(previous) => (draft = previous)}
    >
      <Switch bind:checked={draft.video_transcoding}
        >Allow video transcoding</Switch
      >
      <Switch bind:checked={draft.audio_transcoding}
        >Allow audio transcoding</Switch
      >
    </AutoSaveForm>
  {:else if !error}
    <ContentSkeleton
      label="Loading transcoding settings"
      variant="settings"
      count={2}
    />
  {/if}
  {#if error}
    <Notice role="alert" variant="error">{error}</Notice>
    <Button variant="secondary" size="form" onclick={() => void load()}
      >Retry transcoding settings</Button
    >
  {/if}
</Panel>
