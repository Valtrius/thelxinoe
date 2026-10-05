<script lang="ts">
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import FormField from '../ui/FormField.svelte';
  import ConnectionTestButton from '../ui/ConnectionTestButton.svelte';
  import { formControlClass } from '../ui/styles';
  import { hasServiceUrlBase } from './presentation';
  import type { ManagerService, SupportService } from './feature';

  let {
    service,
    label,
    disabled = false,
    changed,
  }: {
    service: ManagerService | SupportService;
    label: string;
    disabled?: boolean;
    changed: () => Promise<void>;
  } = $props();
  let editing = $state(false);
  let busy = $state(false);
  let port = $state(0);
  let urlBase = $state('');
  let username = $state('');
  let secret = $state('');
  let error = $state('');
  let saved = $state('');
  const path = $derived(`/admin/services/${service.id}/connection`);
  const manager = $derived(
    ['radarr', 'sonarr', 'lidarr'].includes(service.kind),
  );
  async function open() {
    error = '';
    saved = '';
    port = service.port;
    urlBase = service.url_base;
    secret = '';
    busy = true;
    try {
      if (service.kind === 'nzbget')
        username = (await api<{ username: string }>(path)).username;
      editing = true;
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }
  function input() {
    return {
      port,
      url_base: urlBase,
      ...(secret ? { secret } : {}),
      ...(service.kind === 'nzbget' ? { username } : {}),
    };
  }
  async function save() {
    busy = true;
    error = '';
    saved = '';
    try {
      await api(path, 'PUT', input());
      secret = '';
      await changed();
      editing = false;
      saved = 'Connection saved.';
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }
</script>

<section
  class="settings-section min-w-0"
  aria-label={`${label} connection settings`}
>
  {#if !editing}
    <Button
      variant="secondary"
      size="form"
      disabled={disabled || busy}
      loading={busy}
      onclick={() => void open()}>Edit connection</Button
    >
  {:else}
    <form
      class="grid gap-3 [&_label]:m-0"
      aria-label={`${label} API connection`}
      onsubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <h3 class="m-0 text-xs font-semibold">API connection</h3>
      <div class="grid grid-cols-2 gap-3 compact:grid-cols-1">
        <FormField
          >Internal port<input
            class={formControlClass}
            type="number"
            min="1"
            max="65535"
            required
            bind:value={port}
            disabled={disabled || busy}
          /></FormField
        >
        {#if hasServiceUrlBase(service.kind)}
          <FormField
            >URL Base<input
              class={formControlClass}
              bind:value={urlBase}
              disabled={disabled || busy}
              placeholder="/existing-prefix"
            /></FormField
          >
        {/if}
      </div>
      {#if service.kind === 'nzbget'}
        <FormField
          >Login<input
            class={formControlClass}
            bind:value={username}
            maxlength="100"
            autocomplete="username"
            disabled={disabled || busy}
          /></FormField
        >
      {/if}
      <FormField
        >{manager || service.kind !== 'nzbget' ? 'API key' : 'Password'}<input
          class={formControlClass}
          type="password"
          bind:value={secret}
          maxlength="1024"
          autocomplete="new-password"
          placeholder="Keep current credential"
          disabled={disabled || busy}
        /></FormField
      >
      <div class="flex flex-wrap gap-2">
        <ConnectionTestButton
          disabled={disabled || busy}
          test={async () => {
            error = '';
            busy = true;
            try {
              return (
                await api<{ healthy: boolean }>(`${path}/test`, 'POST', input())
              ).healthy;
            } finally {
              busy = false;
            }
          }}
          onError={(failure) => (error = String(failure))}
        />
        <Button
          type="submit"
          size="form"
          disabled={disabled || busy}
          loading={busy}>Save connection</Button
        >
        <Button
          variant="secondary"
          size="form"
          disabled={busy}
          onclick={() => {
            editing = false;
            secret = '';
            error = '';
          }}>Cancel</Button
        >
      </div>
    </form>
  {/if}
  {#if error}<p class="my-2 wrap-anywhere text-xs text-danger" role="alert">
      {error}
    </p>{/if}
  {#if saved}<p class="my-2 text-xs text-muted" role="status">{saved}</p>{/if}
</section>
