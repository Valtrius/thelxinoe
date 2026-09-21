<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  type Policy = {
    service_id: string;
    policy: string;
    window_start: number;
    window_end: number;
    candidate: string | null;
    error: string | null;
  };
  type Update = {
    id: string;
    service_id: string;
    state: string;
    candidate: string | null;
    error: string | null;
    classification?: string;
  };
  const stages: Record<string, string> = {
    queued: 'Waiting to check',
    submitting: 'Starting check',
    preparing: 'Preparing',
    snapshotting: 'Saving appdata',
    preflight: 'Checking compatibility',
    ready: 'Compatible — ready to install',
    'queued-activate': 'Waiting to install',
    'recovery-snapshot': 'Saving recovery copy',
    'isolated-live-validation': 'Validating before activation',
    activating: 'Starting updated service',
    committed: 'Installed',
    'rolled-back': 'Previous service restored',
    blocked: 'Needs attention',
    'recovery-required': 'Recovery required',
    'runtime-failure': 'Updated service needs attention',
    'queued-recover': 'Waiting to restore',
  };
  let services = $state<{ id: string; kind: string }[]>([]);
  let policies = $state<Policy[]>([]),
    updates = $state<Update[]>([]);
  let selected = $state('default'),
    policy = $state('notify'),
    start = $state(0),
    end = $state(0);
  let busy = $state(false),
    message = $state('');
  function choose() {
    const value = policies.find((p) => p.service_id === selected);
    policy = value?.policy ?? 'inherit';
    start = value?.window_start ?? 0;
    end = value?.window_end ?? 0;
  }
  async function refresh() {
    const result = await api<{
      policies: Policy[];
      items: Update[];
      services: typeof services;
    }>('/admin/service-updates');
    policies = result.policies;
    updates = result.items;
    services = result.services;
  }
  async function work(fn: () => Promise<void>) {
    busy = true;
    message = '';
    try {
      await fn();
      await refresh();
    } catch (error) {
      message = String(error);
    } finally {
      busy = false;
    }
  }
  onMount(() => {
    void work(async () => {
      await refresh();
      choose();
    });
    const timer = setInterval(() => {
      if (!busy) void refresh().catch(() => {});
    }, 4000);
    return () => clearInterval(timer);
  });
</script>

<Panel
  class="[&_article]:border-t [&_article]:border-line [&_article]:py-4 [&_button]:my-2 [&_button]:mr-2 [&_code]:wrap-anywhere [&_p]:my-[0.6rem]"
>
  <h2>Service updates</h2>
  <p>
    Stable releases must pass an isolated compatibility check before
    installation. Automatic updates wait for idle services and the maintenance
    window.
  </p>
  {#if message}<p role="status">{message}</p>{/if}
  <form
    class="my-4 grid gap-4"
    onsubmit={(event) => {
      event.preventDefault();
      void work(async () => {
        await api('/admin/service-updates/policy/' + selected, 'POST', {
          policy,
          window_start: start,
          window_end: end,
        });
        message = 'Update policy saved.';
      });
    }}
  >
    <label
      >Update policy for<select bind:value={selected} onchange={choose}
        ><option value="default">Server default</option
        >{#each services as service (service.id)}<option value={service.id}
            >{service.kind}</option
          >{/each}</select
      ></label
    >
    <label
      >Policy<select bind:value={policy}
        >{#if selected !== 'default'}<option value="inherit"
            >Use server default</option
          >{/if}<option value="notify">Notify</option><option value="automatic"
          >Automatic</option
        ><option value="manual">Manual</option></select
      ></label
    >
    <label
      >Maintenance starts (UTC hour)<input
        type="number"
        min="0"
        max="23"
        bind:value={start}
        required
      /></label
    >
    <label
      >Maintenance ends (UTC hour)<input
        type="number"
        min="0"
        max="23"
        bind:value={end}
        required
      /></label
    >
    <p>Equal hours allow the full day. A window may cross midnight.</p>
    <Button type="submit" size="form" disabled={busy}>Save update policy</Button
    >
  </form>
  <Button
    variant="secondary"
    size="form"
    disabled={busy}
    onclick={() =>
      void work(async () => {
        await api('/admin/service-updates/check', 'POST', {});
      })}>Check stable updates</Button
  >
  {#each services as service (service.id)}
    <article>
      <h3>{service.kind}</h3>
      {#each policies.filter((p) => p.service_id === service.id) as p (p.service_id)}
        {#if p.candidate}<details>
            <summary>Discovered stable image</summary><code>{p.candidate}</code>
          </details>{/if}
        {#if p.error}<p role="status">{p.error}</p>{/if}
      {/each}
      <Button
        variant="secondary"
        size="form"
        disabled={busy}
        onclick={() =>
          void work(async () => {
            await api(
              '/admin/service-updates/preflight/' + service.id,
              'POST',
              {},
            );
          })}>Check {service.kind} compatibility</Button
      >
      {#each updates.filter((u) => u.service_id === service.id) as update (update.id)}
        <div class="mt-2 bg-surface p-[0.8rem]">
          <strong>{stages[update.state] ?? update.state}</strong>
          {#if update.classification === 'incompatible'}<p>
              Candidate did not pass compatibility checks.
            </p>{:else if update.classification === 'unable-to-verify'}<p>
              Compatibility could not be verified.
            </p>{/if}
          {#if update.error}<p>
              {update.error}
            </p>{/if}
          {#if update.state === 'ready'}<Button
              variant="secondary"
              size="form"
              disabled={busy}
              onclick={() =>
                void work(async () => {
                  await api(
                    '/admin/service-updates/' + update.id + '/activate',
                    'POST',
                    {},
                  );
                })}>Install verified update</Button
            >{/if}
          {#if ['blocked', 'recovery-required'].includes(update.state)}<Button
              variant="secondary"
              size="form"
              disabled={busy}
              onclick={() =>
                void work(async () => {
                  await api(
                    '/admin/service-updates/' + update.id + '/recover',
                    'POST',
                    {},
                  );
                })}>Recover before activation</Button
            >{/if}
          {#if update.state === 'runtime-failure'}<p>
              The service crossed production activation. Inspect its health
              before planning an explicit restore.
            </p>{/if}
        </div>
      {/each}
    </article>
  {/each}
</Panel>
