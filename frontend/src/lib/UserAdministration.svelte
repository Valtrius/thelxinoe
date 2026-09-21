<script lang="ts">
  import { api, type User } from './api';
  import Button from './ui/Button.svelte';
  import { rowClass } from './ui/styles';
  let { person, currentId, changed } = $props<{
    person: User;
    currentId: string;
    changed: () => Promise<void>;
  }>();
  let expanded = $state(false),
    role = $state<'user' | 'admin'>('user'),
    password = $state(''),
    confirm = $state(''),
    error = $state(''),
    busy = $state(false);
  async function save() {
    busy = true;
    try {
      await api(`/users/${person.id}`, 'PUT', {
        role,
        password: password || null,
      });
      password = '';
      expanded = false;
      if (person.id === currentId) location.reload();
      else await changed();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function remove() {
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

<div class="person">
  <div class={rowClass}>
    <strong>{person.username}</strong><span>{person.role}</span><Button
      variant="secondary"
      size="form"
      onclick={() => {
        role = person.role;
        expanded = !expanded;
      }}>Manage user</Button
    >
  </div>
  {#if expanded}<div class="border border-line p-4">
      <p>
        Saving revokes this user's devices. Deleting removes their personal
        state, playlists and linked credentials. Shared media remains in the
        library.
      </p>
      <label class="my-4 block max-w-120"
        >Role<select bind:value={role}
          ><option value="user">User</option><option value="admin"
            >Administrator</option
          ></select
        ></label
      >
      <label class="my-4 block max-w-120"
        >New password (optional)<input
          type="password"
          autocomplete="new-password"
          minlength="12"
          bind:value={password}
        /></label
      >
      <Button
        variant="secondary"
        size="form"
        disabled={busy}
        onclick={() => void save()}>Save user</Button
      >
      {#if person.id !== currentId}<label class="my-4 block max-w-120"
          >Type {person.username} to confirm deletion<input
            bind:value={confirm}
            autocomplete="off"
          /></label
        ><Button
          variant="secondary"
          size="form"
          disabled={busy || confirm !== person.username}
          onclick={() => void remove()}>Delete user and personal data</Button
        >{/if}
      {#if error}<p role="alert">{error}</p>{/if}
    </div>{/if}
</div>
