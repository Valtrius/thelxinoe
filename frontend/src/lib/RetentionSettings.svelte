<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  type Policy = {
    domain: string;
    enabled: boolean;
    grace_seconds: number;
    exclude_specials: boolean;
    trigger_users: string[];
  };
  type Item = {
    id: string;
    title: string;
    state: string;
    due_at: number;
    error: string | null;
    trigger_user: string | null;
  };
  let policies = $state<Policy[]>([]);
  let users = $state<{ id: string; username: string }[]>([]);
  let roots = $state<
    { id: string; name: string; automatic_unmanaged_deletion: boolean }[]
  >([]);
  let items = $state<Item[]>([]);
  let busy = $state(false),
    message = $state('');
  async function refresh() {
    const result = await api<{
      policies: Policy[];
      users: typeof users;
      roots: typeof roots;
      items: Item[];
    }>('/admin/retention');
    policies = result.policies;
    users = result.users;
    roots = result.roots;
    items = result.items;
  }
  async function work(action: () => Promise<unknown>) {
    busy = true;
    message = '';
    try {
      await action();
      await refresh();
    } catch (error) {
      message = String(error);
    } finally {
      busy = false;
    }
  }
  onMount(() => {
    void work(refresh);
  });
</script>

<section class="panel retention">
  <h2>Watched media retention</h2>
  <p>
    Choose whose watched status can start a grace period. A TV season requires
    every episode to be watched by the same selected user, with confirmed
    mappings and complete aired metadata. Keep and active playback prevent
    deletion.
  </p>
  {#each policies as policy (policy.domain)}
    <fieldset disabled={busy}>
      <legend>{policy.domain === 'movies' ? 'Movies' : 'TV seasons'}</legend>
      <label
        ><input type="checkbox" bind:checked={policy.enabled} /> Enable retention</label
      >
      <label
        >Grace period (hours)<input
          type="number"
          min="0"
          max="8760"
          step="1"
          value={policy.grace_seconds / 3600}
          onchange={(event) =>
            (policy.grace_seconds = Number(event.currentTarget.value) * 3600)}
        /></label
      >
      {#if policy.domain === 'shows'}<label
          ><input type="checkbox" bind:checked={policy.exclude_specials} /> Exclude
          specials (Season 0)</label
        >{/if}
      <p>Retention-trigger users</p>
      {#each users as user (user.id)}<label
          ><input
            type="checkbox"
            checked={policy.trigger_users.includes(user.id)}
            onchange={(event) =>
              (policy.trigger_users = event.currentTarget.checked
                ? [...policy.trigger_users, user.id]
                : policy.trigger_users.filter((id) => id !== user.id))}
          />
          {user.username}</label
        >{/each}
      <button
        onclick={() =>
          work(() =>
            api(`/admin/retention/policy/${policy.domain}`, 'POST', policy),
          )}>Save {policy.domain === 'movies' ? 'movie' : 'TV'} policy</button
      >
    </fieldset>
  {/each}
  <h3>Files without a manager</h3>
  <p>
    Automatic direct deletion requires an explicit opt-in for each library root.
    Otherwise eligible files wait here for an administrator to delete them.
  </p>
  {#each roots as root (root.id)}<label
      ><input
        type="checkbox"
        disabled={busy}
        checked={root.automatic_unmanaged_deletion}
        onchange={(event) =>
          work(() =>
            api(`/admin/retention/root/${root.id}`, 'POST', {
              automatic_unmanaged_deletion: event.currentTarget.checked,
            }),
          )}
      />
      Allow automatic deletion in {root.name}</label
    >{/each}
  <div class="actions">
    <button
      disabled={busy}
      onclick={() => work(() => api('/admin/retention/evaluate', 'POST'))}
      >Evaluate watched media</button
    ><button class="secondary" disabled={busy} onclick={() => work(refresh)}
      >Refresh</button
    >
  </div>
  {#if message}<p role="alert">{message}</p>{/if}
  <h3>Retention queue</h3>
  {#each items as item (item.id)}
    <article>
      <strong>{item.title}</strong>
      <p>
        {item.state} · {item.trigger_user ?? 'Removed user'} · Due {new Date(
          item.due_at * 1000,
        ).toLocaleString()}
      </p>
      {#if item.error}<p>{item.error}</p>{/if}
      {#if item.state === 'pending'}<div class="actions">
          <button
            disabled={busy}
            onclick={() =>
              work(() => api(`/admin/retention/${item.id}/keep`, 'POST'))}
            >Keep</button
          >
          <button
            class="secondary"
            disabled={busy}
            onclick={() =>
              work(() => api(`/admin/retention/${item.id}/cancel`, 'POST'))}
            >Cancel</button
          >
          <button
            class="danger"
            disabled={busy}
            onclick={() => {
              if (
                confirm(
                  `Delete ${item.title} now? This permanently removes its media files.`,
                )
              )
                void work(() =>
                  api(`/admin/retention/${item.id}/delete`, 'POST'),
                );
            }}>Delete now</button
          >
        </div>{/if}
    </article>
  {:else}<p>No retention candidates.</p>{/each}
</section>

<style>
  .retention {
    display: grid;
    gap: 1rem;
  }
  fieldset {
    display: grid;
    gap: 0.7rem;
    border: 1px solid var(--line);
    padding: 1rem;
  }
  label {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 0.5rem;
    margin: 0;
  }
  input[type='checkbox'] {
    width: auto;
  }
  button {
    padding: 0.7rem 1rem;
    background: var(--surface-soft);
    color: var(--foreground);
    justify-self: start;
    font-weight: 600;
    font-size: 13px;
  }
  button.danger {
    background: color-mix(in srgb, var(--danger) 25%, var(--surface));
    color: white;
  }
  input[type='number'] {
    width: 7rem;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  article {
    border-top: 1px solid var(--line);
    padding-top: 1rem;
  }
</style>
