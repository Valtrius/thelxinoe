<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { Accordion } from 'bits-ui';
  import { api, type User } from '../api';
  import { withVerification } from '../authentication';
  import { LatestRequest } from '../latest-request';
  import { isServerUpdateInterruption } from '../server-updates';
  import type { ApprovalUser } from '../services/feature';
  import { captureSession } from '../session';
  import UserAdministration from '../UserAdministration.svelte';
  import Button from '../ui/Button.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import FormField from '../ui/FormField.svelte';
  import Notice from '../ui/Notice.svelte';
  import SettingsCard from '../ui/SettingsCard.svelte';
  import { cardBlockClass, formControlClass } from '../ui/styles';

  let { user, revision } = $props<{ user: User; revision: number }>();
  let users = $state<User[] | null>(null),
    // Who may request without review, keyed by user id. Only people the
    // server lists get a toggle.
    approvals = $state<Record<string, boolean>>({}),
    approvalError = $state(''),
    error = $state(''),
    busy = $state(false);
  let newUsername = $state(''),
    newPassword = $state(''),
    newRole = $state<'admin' | 'user'>('user'),
    expandedUser = $state('');
  const requests = new LatestRequest();
  const ownsSession = captureSession();
  onDestroy(() => requests.invalidate());
  $effect(() => {
    void revision;
    untrack(() => void load());
  });
  async function load() {
    const latest = requests.begin();
    try {
      const [people, approval] = await Promise.all([
        api<{ items: User[] }>('/users'),
        api<{ items: ApprovalUser[] }>('/admin/acquisition/users').then(
          (value) => ({ items: value.items, error: '' }),
          (caught: unknown) => ({
            items: [] as ApprovalUser[],
            error: String(caught),
          }),
        ),
      ]);
      if (!latest()) return;
      users = people.items;
      approvals = Object.fromEntries(
        approval.items.map((entry) => [entry.id, entry.enabled]),
      );
      approvalError = approval.error;
      error = '';
    } catch (caught) {
      if (latest() && !isServerUpdateInterruption(caught))
        error = String(caught);
    }
  }
  async function createUser() {
    busy = true;
    error = '';
    try {
      await withVerification(async () => {
        await api('/users', 'POST', {
          username: newUsername,
          password: newPassword,
          role: newRole,
        });
        if (!ownsSession()) return;
        newUsername = newPassword = '';
        await load();
      });
    } catch (caught) {
      if (ownsSession()) error = String(caught);
    } finally {
      if (ownsSession()) busy = false;
    }
  }
</script>

<SettingsCard
  title="Add user"
  description="New people sign in with a username and password. They can add passkeys or link an identity provider later."
  padded
>
  <form
    class="flex flex-wrap items-end gap-3 compact:flex-col compact:items-stretch [&_label]:m-0 [&_label]:min-w-[min(180px,100%)] [&_label]:flex-1 compact:[&_label]:w-full"
    aria-label="Create user"
    onsubmit={(e) => {
      e.preventDefault();
      void createUser();
    }}
  >
    <FormField
      >Username<input
        class={formControlClass}
        bind:value={newUsername}
        required
        autocomplete="off"
      /></FormField
    ><FormField
      >Password<input
        class={formControlClass}
        bind:value={newPassword}
        type="password"
        required
        minlength="8"
        autocomplete="new-password"
      /></FormField
    ><FormField
      >Role<select class={formControlClass} bind:value={newRole}
        ><option value="user">User</option><option value="admin"
          >Administrator</option
        ></select
      ></FormField
    ><Button size="form" type="submit" disabled={busy}>Add user</Button>
  </form>
  {#if error}<Notice variant="error" role="alert" class="mt-3 mb-0"
      >{error}</Notice
    >{/if}
</SettingsCard>
<SettingsCard
  title="User access"
  description="Roles, automatic request approval, recovery links and account removal for everyone on this server."
>
  {#if !users && !error}<div class={cardBlockClass}>
      <ContentSkeleton label="Loading users" />
    </div>{/if}
  {#if approvalError}<div class={cardBlockClass}>
      <Notice
        variant="error"
        role="alert"
        aria-label="Request approval settings load error"
        class="mb-0"
        >Failed to load request approval settings. {approvalError}</Notice
      >
    </div>{/if}
  <Accordion.Root
    type="single"
    bind:value={expandedUser}
    class="divide-y divide-line"
  >
    {#each users ?? [] as person (person.id)}<UserAdministration
        {person}
        approval={approvals[person.id]}
        currentId={user.id}
        changed={load}
        close={() => (expandedUser = '')}
      />{/each}
  </Accordion.Root>
</SettingsCard>
