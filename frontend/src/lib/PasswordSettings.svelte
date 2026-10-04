<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { api } from './api';
  import { withVerification } from './authentication';
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
    saved = $state(false);

  async function save() {
    error = '';
    saved = false;
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
        });
      };
      if (hasPassword && !totp && current) await action();
      else if (!(await withVerification(action))) return;
      current = password = confirmation = '';
      saved = true;
      changed();
    } catch (caught) {
      error = String(caught);
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
    <Button type="submit" size="form" disabled={busy}>
      {busy ? 'Changing password…' : 'Change password'}
    </Button>
  </form>
  {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
  {#if saved}<p role="status">
      Password changed. Other devices have been signed out.
    </p>{/if}
</Panel>
