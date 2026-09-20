<script lang="ts">
  import { api, type User } from './api';
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
  <div class="row">
    <strong>{person.username}</strong><span>{person.role}</span><button
      class="secondary"
      onclick={() => {
        role = person.role;
        expanded = !expanded;
      }}>Manage user</button
    >
  </div>
  {#if expanded}<div class="editor">
      <p>
        Saving revokes this user's devices. Deleting removes their personal
        state, playlists and linked credentials. Shared media remains in the
        library.
      </p>
      <label
        >Role<select bind:value={role}
          ><option value="user">User</option><option value="admin"
            >Administrator</option
          ></select
        ></label
      >
      <label
        >New password (optional)<input
          type="password"
          autocomplete="new-password"
          minlength="12"
          bind:value={password}
        /></label
      >
      <button class="secondary" disabled={busy} onclick={() => void save()}
        >Save user</button
      >
      {#if person.id !== currentId}<label
          >Type {person.username} to confirm deletion<input
            bind:value={confirm}
            autocomplete="off"
          /></label
        ><button
          class="secondary"
          disabled={busy || confirm !== person.username}
          onclick={() => void remove()}>Delete user and personal data</button
        >{/if}
      {#if error}<p role="alert">{error}</p>{/if}
    </div>{/if}
</div>

<style>
  .editor {
    padding: 1rem;
    border: 1px solid var(--line);
  }
  label {
    display: block;
    margin: 1rem 0;
    max-width: 30rem;
  }
</style>
