<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type User } from './api';
  import { withVerification } from './authentication';
  import Button from './ui/Button.svelte';
  import Notice from './ui/Notice.svelte';
  let { request, user, completed } = $props<{
    request: string;
    user: User;
    completed: () => void;
  }>();
  let name = $state(''),
    busy = $state(false),
    error = $state(''),
    approved = $state(false),
    verifying = $state<string | null>(null);
  onMount(() => {
    void api<{ name: string; verifying: string | null }>(
      '/auth/desktop/info?request=' + encodeURIComponent(request),
    )
      .then((v) => {
        name = v.name;
        verifying = v.verifying;
      })
      .catch((e) => (error = String(e)));
  });
  async function approve() {
    busy = true;
    error = '';
    try {
      await withVerification(async () => {
        await api('/auth/desktop/approve', 'POST', { request });
        approved = true;
      });
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = false;
    }
  }
</script>

<h1>{verifying ? 'Verify desktop' : 'Connect desktop'}</h1>
{#if approved}<p role="status">Desktop approved</p>
  <Button onclick={completed}>Done</Button>{:else if name}<p>
    {verifying ? 'Verify' : 'Sign in to'} <strong>{name}</strong> as
    <strong>{verifying ?? user.username}</strong>
    on
    <strong>{location.host}</strong>.
  </p>
  <Button
    disabled={busy || (!!verifying && verifying !== user.username)}
    onclick={() => approve()}>Approve desktop</Button
  ><Button variant="secondary" disabled={busy} onclick={completed}
    >Cancel</Button
  >{/if}
{#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
