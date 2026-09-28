<script lang="ts">
  import { desktopUpdates, desktopUpdate } from './desktop-updates';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  let policy = $state<'notify' | 'automatic'>('notify');
  let initialized = $state(false);
  let error = $state('');
  const busy = $derived(
    $desktopUpdates &&
      ['checking', 'downloading', 'installing'].includes($desktopUpdates.phase),
  );
  $effect(() => {
    if ($desktopUpdates && !initialized) {
      policy = $desktopUpdates.policy;
      initialized = true;
    }
  });
  async function act(action: 'check' | 'download' | 'install') {
    error = '';
    try {
      await desktopUpdate(action);
    } catch (e) {
      error = String(e);
    }
  }
</script>

<Panel aria-label="Desktop updates">
  <div class="grid gap-4">
    <h2>Desktop updates</h2>
    {#if $desktopUpdates}
      <p>Installed desktop: {$desktopUpdates.installed}</p>
      <AutoSaveForm
        label="Desktop update settings"
        value={{ policy }}
        onRevert={(previous) => (policy = previous.policy)}
        onsave={(value) => desktopUpdate('policy', value)}
      >
        {#snippet children(save)}
          <ExclusiveChoiceGroup
            ariaLabel="Desktop update policy"
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
        {/snippet}
      </AutoSaveForm>
      <p class="text-xs text-muted">
        Checks run automatically. Automatic also downloads updates; installation
        waits for you to restart when playback has stopped.
      </p>
      <Button
        variant="secondary"
        size="form"
        disabled={Boolean(busy)}
        onclick={() => act('check')}>Check desktop release</Button
      >
      {#if $desktopUpdates.release}
        <h3>Version {$desktopUpdates.release.version}</h3>
        <p class="whitespace-pre-wrap">{$desktopUpdates.release.notes}</p>
        {#if $desktopUpdates.phase === 'ready'}
          <p>
            Downloaded and verified. Installation closes and reopens Thelxinoe.
          </p>
          <Button size="form" onclick={() => act('install')}
            >Restart and install desktop update</Button
          >
        {:else}
          <Button
            size="form"
            disabled={Boolean(busy)}
            onclick={() => act('download')}>Download desktop update</Button
          >
        {/if}
      {:else if $desktopUpdates.checked_at && !$desktopUpdates.error && !busy}
        <p>Desktop is up to date.</p>
      {/if}
      {#if $desktopUpdates.phase === 'downloading'}
        <progress
          class="w-full"
          value={$desktopUpdates.received}
          max={$desktopUpdates.total || 1}
          aria-label="Desktop update download"
        ></progress>
        <p role="status">Downloading and verifying the installer…</p>
      {:else if $desktopUpdates.phase === 'checking'}<p role="status">
          Checking for a desktop release…
        </p>
      {:else if $desktopUpdates.phase === 'installing'}<p role="status">
          Starting the installer…
        </p>{/if}
      {#if $desktopUpdates.checked_at}<p class="text-xs text-muted">
          Last check: {new Date(
            $desktopUpdates.checked_at * 1000,
          ).toLocaleString()}
        </p>{/if}
    {/if}
    {#if error || $desktopUpdates?.error}<p
        role="status"
        class="text-sm text-danger"
      >
        {error || $desktopUpdates?.error}
      </p>{/if}
  </div>
</Panel>
