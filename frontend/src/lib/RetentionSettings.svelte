<script lang="ts">
  import Switch from './ui/Switch.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  let { timezone, timeFormat } = $props<{
    timezone: string;
    timeFormat: '12h' | '24h';
  }>();
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

<Panel class="grid gap-4">
  <h2>Watched media retention</h2>
  <p>
    Choose whose watched status can start a grace period. A TV season requires
    every episode to be watched by the same selected user, with confirmed
    mappings and complete aired metadata. Keep and active playback prevent
    deletion.
  </p>
  {#each policies as policy (policy.domain)}
    <fieldset class="grid gap-[0.7rem] border border-line p-4" disabled={busy}>
      <legend>{policy.domain === 'movies' ? 'Movies' : 'TV seasons'}</legend>
      <Switch bind:checked={policy.enabled}>Enable retention</Switch>
      <label class="m-0 flex-row items-center gap-2"
        >Grace period (hours)<input
          class="w-28"
          type="number"
          min="0"
          max="8760"
          step="1"
          value={policy.grace_seconds / 3600}
          onchange={(event) =>
            (policy.grace_seconds = Number(event.currentTarget.value) * 3600)}
        /></label
      >
      {#if policy.domain === 'shows'}<Switch
          bind:checked={policy.exclude_specials}
          >Exclude specials (Season 0)</Switch
        >{/if}
      <p>Retention-trigger users</p>
      {#each users as user (user.id)}<Switch
          checked={policy.trigger_users.includes(user.id)}
          onCheckedChange={(checked) =>
            (policy.trigger_users = checked
              ? [...policy.trigger_users, user.id]
              : policy.trigger_users.filter((id) => id !== user.id))}
        >
          {user.username}</Switch
        >{/each}
      <Button
        variant="secondary"
        size="form"
        class="justify-self-start px-4 py-[0.7rem] text-[13px]"
        onclick={() =>
          work(() =>
            api(`/admin/retention/policy/${policy.domain}`, 'POST', policy),
          )}>Save {policy.domain === 'movies' ? 'movie' : 'TV'} policy</Button
      >
    </fieldset>
  {/each}
  <h3>Files without a manager</h3>
  <p>
    Automatic direct deletion requires an explicit opt-in for each library root.
    Otherwise eligible files wait here for an administrator to delete them.
  </p>
  {#each roots as root (root.id)}<Switch
      disabled={busy}
      checked={root.automatic_unmanaged_deletion}
      onCheckedChange={(checked) =>
        work(() =>
          api(`/admin/retention/root/${root.id}`, 'POST', {
            automatic_unmanaged_deletion: checked,
          }),
        )}
    >
      Allow automatic deletion in {root.name}</Switch
    >{/each}
  <div class="flex flex-wrap gap-2">
    <Button
      variant="secondary"
      size="form"
      class="justify-self-start px-4 py-[0.7rem] text-[13px]"
      disabled={busy}
      onclick={() => work(() => api('/admin/retention/evaluate', 'POST'))}
      >Evaluate watched media</Button
    ><Button
      variant="secondary"
      size="form"
      class="justify-self-start px-4 py-[0.7rem] text-[13px]"
      disabled={busy}
      onclick={() => work(refresh)}>Refresh</Button
    >
  </div>
  {#if message}<p role="alert">{message}</p>{/if}
  <h3>Retention queue</h3>
  {#each items as item (item.id)}
    <article class="border-t border-line pt-4">
      <strong>{item.title}</strong>
      <p>
        {item.state} · {item.trigger_user ?? 'Removed user'} · Due {new Date(
          item.due_at * 1000,
        ).toLocaleString(undefined, {
          timeZone: timezone,
          hour12: timeFormat === '12h',
        })}
      </p>
      {#if item.error}<p>{item.error}</p>{/if}
      {#if item.state === 'pending'}<div class="flex flex-wrap gap-2">
          <Button
            variant="secondary"
            size="form"
            class="justify-self-start px-4 py-[0.7rem] text-[13px]"
            disabled={busy}
            onclick={() =>
              work(() => api(`/admin/retention/${item.id}/keep`, 'POST'))}
            >Keep</Button
          >
          <Button
            variant="secondary"
            size="form"
            class="justify-self-start px-4 py-[0.7rem] text-[13px]"
            disabled={busy}
            onclick={() =>
              work(() => api(`/admin/retention/${item.id}/cancel`, 'POST'))}
            >Cancel</Button
          >
          <Button
            variant="danger"
            size="form"
            class="justify-self-start px-4 py-[0.7rem] text-[13px]"
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
            }}>Delete now</Button
          >
        </div>{/if}
    </article>
  {:else}<p>No retention candidates.</p>{/each}
</Panel>
