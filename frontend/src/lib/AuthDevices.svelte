<script lang="ts">
  import { onMount } from 'svelte';
  import { Laptop, Tv } from '@lucide/svelte';
  import { api } from './api';
  import { withVerification } from './authentication';
  import { captureSession } from './session';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import Button from './ui/Button.svelte';
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { cardBlockClass, formControlClass } from './ui/styles';
  import { twMerge } from 'tailwind-merge';
  import { reveal } from './ui/reveal';
  type Device = { id: string; name: string; last_seen: number };
  const ids = $props.id();
  let devices = $state<Device[]>([]),
    clients = $state<Device[]>([]),
    name = $state('');
  let busy = $state(false),
    error = $state(''),
    password = $state('');
  onMount(() => {
    void load().catch((e) => (error = String(e)));
  });
  async function load() {
    const owns = captureSession();
    const [remembered, app] = await Promise.all([
      api<{ items: Device[] }>('/auth/devices'),
      api<{ items: Device[] }>('/me/auth/client-passwords'),
    ]);
    if (owns()) {
      devices = remembered.items;
      clients = app.items;
    }
  }
  async function act(action: () => Promise<void>) {
    const owns = captureSession();
    busy = true;
    error = '';
    try {
      await withVerification(action);
      if (owns()) await load();
    } catch (caught) {
      if (owns()) error = String(caught);
    } finally {
      if (owns()) busy = false;
    }
  }
</script>

<SettingsCard
  title="Remembered devices"
  description="Devices you chose to remember at sign-in skip repeated verification."
>
  {#each devices as device (device.id)}
    <SettingRow label={device.name}>
      {#snippet lead()}
        <Laptop size={16} class="shrink-0 text-muted" aria-hidden="true" />
      {/snippet}
      <Button
        variant="ghost"
        size="form"
        disabled={busy}
        aria-label={`Forget ${device.name}`}
        onclick={() =>
          act(async () => {
            await api(`/auth/devices/${device.id}`, 'DELETE');
          })}>Forget device</Button
      >
    </SettingRow>
  {:else}
    <p class="m-0 {cardBlockClass} text-xs text-muted">No remembered devices</p>
  {/each}
</SettingsCard>
<SettingsCard
  title="Client passwords"
  description="For compatible media clients. These passwords bypass TOTP and can only access the Jellyfin-compatible API."
>
  {#each clients as client (client.id)}
    <SettingRow label={client.name}>
      {#snippet lead()}
        <Tv size={16} class="shrink-0 text-muted" aria-hidden="true" />
      {/snippet}
      <Button
        variant="ghost"
        size="form"
        disabled={busy}
        aria-label={`Revoke ${client.name}`}
        onclick={() =>
          act(async () => {
            await api(`/me/auth/client-passwords/${client.id}`, 'DELETE');
          })}>Revoke</Button
      >
    </SettingRow>
  {:else}
    <p class="m-0 {cardBlockClass} text-xs text-muted">
      No client passwords yet.
    </p>
  {/each}
  {#if password}
    <div
      class="{cardBlockClass} grid max-w-120 gap-2 bg-surface-soft"
      transition:reveal
    >
      <FormField class="mb-0"
        >New client password<input
          class={formControlClass}
          value={password}
          readonly
        /></FormField
      >
      <p class="m-0 text-xs text-muted">
        Copy this password now. It is shown once.
      </p>
      <div>
        <Button variant="secondary" size="form" onclick={() => (password = '')}
          >Done</Button
        >
      </div>
    </div>
  {/if}
  {#if error}
    <div class={cardBlockClass}>
      <Notice role="alert" variant="error" class="mb-0">{error}</Notice>
    </div>
  {/if}
  {#snippet footer()}
    <form
      class="flex flex-wrap items-end gap-2"
      onsubmit={(e) => {
        e.preventDefault();
        void act(async () => {
          password = (
            await api<{ password: string }>(
              '/me/auth/client-passwords',
              'POST',
              {
                name,
              },
            )
          ).password;
          name = '';
        });
      }}
    >
      <label for={`${ids}-client`} class="sr-only">Client name</label>
      <input
        id={`${ids}-client`}
        class={twMerge(formControlClass, 'w-56 compact:w-full')}
        placeholder="Client name, for example Living room TV"
        bind:value={name}
        maxlength="100"
        required
        disabled={busy}
      />
      <Button type="submit" size="form" disabled={busy}
        >Create client password</Button
      >
    </form>
  {/snippet}
</SettingsCard>
