<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { api, type User } from './api';
  import { Accordion } from 'bits-ui';
  import { ChevronDown } from '@lucide/svelte';
  import Button from './ui/Button.svelte';
  import { badgeClass } from './ui/styles';
  let { person, currentId, changed, close } = $props<{
    person: User;
    currentId: string;
    changed: () => Promise<void>;
    close: () => void;
  }>();
  let role = $state<'user' | 'admin'>('user'),
    password = $state(''),
    confirm = $state(''),
    error = $state(''),
    busy = $state(false);
  async function save() {
    error = '';
    busy = true;
    try {
      await api(`/users/${person.id}`, 'PUT', {
        role,
        password: password || null,
      });
      password = '';
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
      await api(`/users/${person.id}`, 'DELETE');
      await changed();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Accordion.Item value={person.id} class="person border-b border-line">
  <Accordion.Header>
    <Accordion.Trigger
      class="group flex w-full items-center gap-3 py-4 text-left hover:text-accent"
      disabled={busy}
      onclick={() => {
        role = person.role;
        error = '';
      }}
      ><strong class="min-w-0 flex-1 truncate">{person.username}</strong>
      <span class={badgeClass}>{person.role}</span>
      <ChevronDown
        class="size-4 shrink-0 text-muted transition-transform group-data-[state=open]:rotate-180"
      /></Accordion.Trigger
    >
  </Accordion.Header>
  <Accordion.Content class="overflow-hidden pb-4">
    <form
      onsubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <p>
        Saving revokes this user's devices. Deleting removes their personal
        state, playlists and linked credentials. Shared media remains in the
        library.
      </p>
      <FormField class="my-4 block max-w-120"
        >Role<select class={formControlClass} bind:value={role}
          ><option value="user">User</option><option value="admin"
            >Administrator</option
          ></select
        ></FormField
      >
      <FormField class="my-4 block max-w-120"
        >New password (optional)<input
          class={formControlClass}
          type="password"
          autocomplete="new-password"
          minlength="8"
          bind:value={password}
        /></FormField
      >
      <Button variant="secondary" size="form" type="submit" disabled={busy}
        >Save user</Button
      >
    </form>
    {#if person.id !== currentId}<FormField class="my-4 block max-w-120"
        >Type {person.username} to confirm deletion<input
          class={formControlClass}
          bind:value={confirm}
          autocomplete="off"
        /></FormField
      ><Button
        variant="secondary"
        size="form"
        disabled={busy || confirm !== person.username}
        onclick={() => void remove()}>Delete user and personal data</Button
      >{/if}
    {#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
  </Accordion.Content>
</Accordion.Item>
