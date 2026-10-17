<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { api, serverUrl, type User } from './api';
  import { withVerification } from './authentication';
  import { Accordion } from 'bits-ui';
  import { ChevronDown } from '@lucide/svelte';
  import Button from './ui/Button.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import Switch from './ui/Switch.svelte';
  import { cardBlockClass, formControlClass } from './ui/styles';
  import { twMerge } from 'tailwind-merge';
  import { reveal } from './ui/reveal';
  // `approval` is undefined for people the server does not list for
  // automatic request approval; the row is hidden for them.
  let { person, approval, currentId, changed, close } = $props<{
    person: User;
    approval?: boolean;
    currentId: string;
    changed: () => Promise<void>;
    close: () => void;
  }>();
  const ids = $props.id();
  let role = $state<'user' | 'admin'>('user'),
    recoveryUrl = $state(''),
    confirm = $state(''),
    error = $state(''),
    busy = $state(false);
  async function save() {
    error = '';
    busy = true;
    try {
      const executed = await withVerification(async () => {
        await api(`/users/${person.id}`, 'PUT', {
          role,
        });
      });
      if (!executed) return;
      close();
      if (person.id === currentId) location.reload();
      else await changed();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function remove() {
    error = '';
    busy = true;
    try {
      await withVerification(async () => {
        await api(`/users/${person.id}`, 'DELETE');
        await changed();
      });
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function approve(enabled: boolean) {
    error = '';
    busy = true;
    try {
      await api(`/admin/acquisition/users/${person.id}`, 'PUT', { enabled });
      await changed();
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = false;
    }
  }
  async function recover() {
    busy = true;
    error = '';
    try {
      await withVerification(async () => {
        recoveryUrl = (
          await api<{ url: string }>(`/users/${person.id}/recovery`, 'POST')
        ).url;
        recoveryUrl = new URL(
          recoveryUrl,
          serverUrl() || location.origin,
        ).toString();
      });
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = false;
    }
  }
</script>

<Accordion.Item value={person.id} class="person">
  <Accordion.Header>
    <Accordion.Trigger
      class="group flex w-full items-center gap-3 px-5.5 py-3.5 text-left hover:bg-surface-soft compact:px-4"
      disabled={busy}
      onclick={() => {
        role = person.role;
        error = '';
      }}
    >
      <span
        class="grid size-8 shrink-0 place-items-center rounded-full border border-line-strong bg-surface-strong text-xs font-semibold text-accent"
        aria-hidden="true">{person.username[0]?.toUpperCase() ?? '?'}</span
      >
      <strong class="min-w-0 flex-1 truncate text-[13px] font-[550]"
        >{person.username}</strong
      >
      <StatusChip
        tone={person.role === 'admin' ? 'info' : 'neutral'}
        dot={false}>{person.role}</StatusChip
      >
      {#if person.id === currentId}<StatusChip dot={false}>You</StatusChip>{/if}
      <ChevronDown
        class="size-4 shrink-0 text-muted transition-transform group-data-[state=open]:rotate-180"
      />
    </Accordion.Trigger>
  </Accordion.Header>
  <Accordion.Content forceMount>
    {#snippet child({ props, open })}
      {#if open}
        <div {...props} transition:reveal>
          <div
            class="divide-y divide-line border-t border-line bg-surface-soft"
          >
            <form
              onsubmit={(event) => {
                event.preventDefault();
                void save();
              }}
            >
              <SettingRow
                label="Role"
                description="Saving revokes this user's devices. Deleting removes their personal state, playlists and linked credentials; shared media remains in the library."
                for={`${ids}-role`}
              >
                <select
                  id={`${ids}-role`}
                  class={twMerge(formControlClass, 'w-44')}
                  bind:value={role}
                  ><option value="user">User</option><option value="admin"
                    >Administrator</option
                  ></select
                >
                <Button
                  variant="secondary"
                  size="form"
                  type="submit"
                  disabled={busy}>Save user</Button
                >
              </SettingRow>
            </form>
            {#if approval !== undefined}
              <SettingRow
                label="Automatic request approval"
                description="Seerr requests from this person are accepted without review."
              >
                {#snippet children({ labelId })}
                  <Switch
                    labelledby={labelId}
                    checked={approval}
                    disabled={busy}
                    onCheckedChange={(enabled) => void approve(enabled)}
                  />
                {/snippet}
              </SettingRow>
            {/if}
            {#if person.id !== currentId}
              <SettingRow
                label="Recovery link"
                description="Lets this person set up sign-in again. Their previous sign-in methods and devices are revoked; the link expires in 10 minutes."
              >
                <Button
                  variant="secondary"
                  size="form"
                  disabled={busy}
                  onclick={() => recover()}>Create recovery link</Button
                >
              </SettingRow>
            {/if}
            {#if recoveryUrl}
              <div class="{cardBlockClass} grid gap-2" transition:reveal>
                <FormField class="mb-0 max-w-none"
                  >Recovery link<input
                    class={formControlClass}
                    value={recoveryUrl}
                    readonly
                  /></FormField
                >
                <p class="m-0 text-xs text-muted">
                  Expires in 10 minutes. The user's previous sign-in methods and
                  devices have been revoked.
                </p>
              </div>
            {/if}
            {#if person.id !== currentId}
              <SettingRow
                label="Delete user and personal data"
                description={`Type ${person.username} to confirm. Removes their personal state, playlists and linked credentials.`}
                for={`${ids}-confirm`}
              >
                <input
                  id={`${ids}-confirm`}
                  class={twMerge(formControlClass, 'w-44')}
                  aria-label={`Type ${person.username} to confirm deletion`}
                  placeholder={person.username}
                  bind:value={confirm}
                  autocomplete="off"
                />
                <Button
                  variant="danger"
                  size="form"
                  disabled={busy || confirm !== person.username}
                  onclick={() => void remove()}
                  >Delete user and personal data</Button
                >
              </SettingRow>
            {/if}
            {#if error}
              <div class={cardBlockClass}>
                <Notice variant="error" role="alert" class="mb-0"
                  >{error}</Notice
                >
              </div>
            {/if}
          </div>
        </div>
      {/if}
    {/snippet}
  </Accordion.Content>
</Accordion.Item>
