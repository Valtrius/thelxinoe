<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import { registerPasskey } from './authentication';
  import Button from './ui/Button.svelte';
  import FormField from './ui/FormField.svelte';
  import Notice from './ui/Notice.svelte';
  import { formControlClass } from './ui/styles';
  let { token, completed } = $props<{ token: string; completed: () => void }>();
  let username = $state(''),
    password = $state(''),
    confirmation = $state(''),
    busy = $state(false),
    error = $state('');
  onMount(() => {
    history.replaceState(null, '', location.pathname + location.search);
    void api<{ username: string }>('/auth/recovery/info', 'POST', { token })
      .then((v) => (username = v.username))
      .catch((e) => (error = String(e)));
  });
  async function save(passkey: boolean) {
    busy = true;
    error = '';
    try {
      if (passkey) await registerPasskey('Passkey', token);
      else {
        if (password !== confirmation)
          throw new Error('Passwords do not match.');
        await api('/auth/recovery/enroll', 'POST', { token, password });
      }
      password = confirmation = '';
      completed();
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = false;
    }
  }
</script>

<h1>Recover account</h1>
{#if username}<p>{username}</p>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      void save(false);
    }}
  >
    <FormField
      >New password<input
        class={formControlClass}
        bind:value={password}
        type="password"
        autocomplete="new-password"
        required
        minlength="8"
        disabled={busy}
      /></FormField
    >
    <FormField
      >Confirm password<input
        class={formControlClass}
        bind:value={confirmation}
        type="password"
        autocomplete="new-password"
        required
        minlength="8"
        disabled={busy}
      /></FormField
    >
    <Button type="submit" disabled={busy}>Save sign-in method</Button>
  </form>
  {#if window.isSecureContext && location.hostname !== '127.0.0.1'}<Button
      variant="secondary"
      disabled={busy}
      onclick={() => save(true)}>Enroll a passkey</Button
    >{/if}{/if}
{#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
