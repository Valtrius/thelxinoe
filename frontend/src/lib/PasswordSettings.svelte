<script lang="ts">
  import { onMount } from 'svelte';
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { api } from './api';
  import { errorMessage, withVerification } from './authentication';
  import { captureSession } from './session';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';

  let {
    changed,
    hasPassword = true,
    totp = false,
    fresh = false,
  } = $props<{
    changed: () => void;
    hasPassword?: boolean;
    totp?: boolean;
    fresh?: boolean;
  }>();
  let current = $state(''),
    password = $state(''),
    confirmation = $state(''),
    busy = $state(false),
    error = $state(''),
    saved = $state('');
  // Client passwords keep media apps signed in after a password change unless revoked too.
  let clients = $state<{ id: string; name: string }[]>([]),
    revokeClients = $state(false);
  onMount(() => void loadClients());
  async function loadClients() {
    const owns = captureSession();
    try {
      const { items } = await api<{ items: { id: string; name: string }[] }>(
        '/me/auth/client-passwords',
      );
      if (owns()) clients = items;
    } catch {
      // The option stays hidden; client passwords are managed under Devices.
    }
  }

  async function save() {
    error = '';
    saved = '';
    if (password !== confirmation) {
      error = 'The new passwords do not match.';
      return;
    }
    busy = true;
    try {
      const action = async () => {
        await api('/me/password', 'PUT', {
          current_password: current,
          new_password: password,
          revoke_client_passwords: revokeClients && clients.length > 0,
        });
      };
      if (hasPassword && !totp && current) await action();
      else if (!(await withVerification(action))) return;
      saved =
        revokeClients && clients.length
          ? 'Password changed. Other devices have been signed out and client passwords revoked; give your media apps a new client password under Devices.'
          : clients.length
            ? 'Password changed. Other devices have been signed out. Media apps using a client password stay signed in; manage them under Devices.'
            : 'Password changed. Other devices have been signed out.';
      current = password = confirmation = '';
      revokeClients = false;
      changed();
      void loadClients();
    } catch (caught) {
      error = errorMessage(caught);
    } finally {
      busy = false;
    }
  }
</script>

<Panel>
  <h2>{hasPassword ? 'Change password' : 'Set password'}</h2>
  <p class="text-muted">
    Choose a password with at least 8 characters. Your other devices will be
    signed out; this device stays connected.
  </p>
  <form
    class="max-w-120"
    onsubmit={(event) => {
      event.preventDefault();
      void save();
    }}
  >
    {#if hasPassword && !totp && !fresh}<FormField
        >Current password<input
          class={formControlClass}
          type="password"
          bind:value={current}
          required
          autocomplete="current-password"
          disabled={busy}
        /></FormField
      >{/if}
    <FormField
      >New password<input
        class={formControlClass}
        type="password"
        bind:value={password}
        required
        minlength="8"
        autocomplete="new-password"
        disabled={busy}
      /></FormField
    >
    <FormField
      >Confirm new password<input
        class={formControlClass}
        type="password"
        bind:value={confirmation}
        required
        minlength="8"
        autocomplete="new-password"
        disabled={busy}
      /></FormField
    >
    {#if clients.length}<FormField
        ><input
          type="checkbox"
          class={formControlClass}
          bind:checked={revokeClients}
          disabled={busy}
        />Also revoke client passwords</FormField
      >
      <p class="-mt-2 mb-4 text-xs text-muted">
        Media apps signed in with {clients.map((c) => c.name).join(', ')} keep working
        after a password change unless you revoke their client passwords. Revoked
        apps need a new client password.
      </p>{/if}
    <Button type="submit" size="form" disabled={busy}>
      {busy ? 'Changing password…' : 'Change password'}
    </Button>
  </form>
  {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
  {#if saved}<p role="status">{saved}</p>{/if}
</Panel>
