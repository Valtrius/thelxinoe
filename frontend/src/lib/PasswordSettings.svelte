<script lang="ts">
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { errorClass } from './ui/styles';

  let { changed } = $props<{ changed: () => void }>();
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
      await api('/me/password', 'PUT', {
        current_password: current,
        new_password: password,
      });
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
  <h2>Change password</h2>
  <p class="text-muted">
    Choose a password with at least 12 characters. Your other devices will be
    signed out; this device stays connected.
  </p>
  <form
    class="max-w-120"
    onsubmit={(event) => {
      event.preventDefault();
      void save();
    }}
  >
    <label
      >Current password<input
        type="password"
        bind:value={current}
        required
        autocomplete="current-password"
        disabled={busy}
      /></label
    >
    <label
      >New password<input
        type="password"
        bind:value={password}
        required
        minlength="12"
        autocomplete="new-password"
        disabled={busy}
      /></label
    >
    <label
      >Confirm new password<input
        type="password"
        bind:value={confirmation}
        required
        minlength="12"
        autocomplete="new-password"
        disabled={busy}
      /></label
    >
    <Button type="submit" size="form" disabled={busy}>
      {busy ? 'Changing password…' : 'Change password'}
    </Button>
  </form>
  {#if error}<p role="alert" class={errorClass}>{error}</p>{/if}
  {#if saved}<p role="status">
      Password changed. Other devices have been signed out.
    </p>{/if}
</Panel>
