<script lang="ts">
  import { onMount } from 'svelte';
  import { api, ApiError, type User } from './api';
  import { errorMessage, withVerification } from './authentication';
  import Button from './ui/Button.svelte';
  import FormField from './ui/FormField.svelte';
  import Notice from './ui/Notice.svelte';
  import { formControlClass } from './ui/styles';
  type Request = {
    name: string;
    verifying: string | null;
    requested_from: string;
    requested_at: number;
    same_network: boolean;
  };
  let { request, user, completed } = $props<{
    request: string;
    user: User;
    completed: () => void;
  }>();
  let info = $state<Request | null>(null),
    code = $state(''),
    busy = $state(false),
    error = $state(''),
    ended = $state(false),
    approved = $state(false);
  onMount(() => {
    void api<Request>(
      '/auth/desktop/info?request=' + encodeURIComponent(request),
    )
      .then((v) => (info = v))
      .catch((e) => {
        error = errorMessage(e);
        ended = true;
      });
  });
  const requestedAt = $derived(
    info
      ? new Date(info.requested_at * 1000).toLocaleTimeString([], {
          hour: '2-digit',
          minute: '2-digit',
        })
      : '',
  );
  async function approve() {
    error = '';
    if (code.replace(/[^0-9a-z]/gi, '').length !== 8) {
      error = 'Enter the 8-character code shown in the desktop app.';
      return;
    }
    busy = true;
    try {
      await withVerification(async () => {
        await api('/auth/desktop/approve', 'POST', { request, code });
        approved = true;
      });
    } catch (caught) {
      error = errorMessage(caught);
      if (caught instanceof ApiError && caught.code === 'desktop_request_ended')
        ended = true;
    } finally {
      busy = false;
    }
  }
</script>

<h1>{info?.verifying ? 'Verify desktop' : 'Connect desktop'}</h1>
{#if approved}<p role="status">
    Desktop approved. You can return to the desktop app.
  </p>
  <Button onclick={completed}>Done</Button>{:else if info && !ended}<p>
    {info.verifying ? 'Verify' : 'Sign in to'} <strong>{info.name}</strong> as
    <strong>{info.verifying ?? user.username}</strong>
    on
    <strong>{location.host}</strong>.
  </p>
  <p class="text-xs text-muted">
    Requested at {requestedAt} from {info.requested_from}.
  </p>
  {#if info.verifying && info.verifying !== user.username}<Notice tone="warning"
      ><p>
        Sign in as {info.verifying} to verify this desktop.
      </p></Notice
    >{/if}
  {#if !info.same_network}<Notice tone="warning">
      <p>
        This request came from a different network address than yours. Only
        continue if you started it yourself on that desktop.
      </p>
    </Notice>{/if}
  <form
    novalidate
    onsubmit={(event) => {
      event.preventDefault();
      void approve();
    }}
  >
    <FormField
      >Code shown in the desktop app<input
        class="{formControlClass} font-mono tracking-[0.2em] uppercase"
        bind:value={code}
        autocomplete="off"
        autocapitalize="characters"
        spellcheck="false"
        maxlength="9"
        placeholder="XXXX-XXXX"
        required
        disabled={busy}
      /></FormField
    >
    <p class="mt-0 text-xs text-muted">
      Never approve a request you didn't start. Someone who sends you this link
      is trying to get into your account.
    </p>
    <div class="flex gap-2">
      <Button
        type="submit"
        loading={busy}
        disabled={!!info.verifying && info.verifying !== user.username}
        >Approve desktop</Button
      ><Button variant="secondary" disabled={busy} onclick={completed}
        >Cancel</Button
      >
    </div>
  </form>
{:else if ended}<Button variant="secondary" onclick={completed}>Close</Button
  >{/if}
{#if error}<Notice variant="error" role="alert" class="mt-4">{error}</Notice
  >{/if}
