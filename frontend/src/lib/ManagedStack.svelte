<script lang="ts">
  import { api } from './api';
  import { onMount } from 'svelte';
  import Switch from './providers/components/ui/Switch.svelte';
  type Service = {
    id: string;
    kind: string;
    name: string;
    phase: string;
    image: string;
    drift: boolean;
    running: boolean;
    error: string | null;
    transfer_pending: boolean;
  };
  type Provision = {
    id: string;
    kind: string;
    state: string;
    host_port: number;
    native_url: string;
    origin: string;
    service_id: string | null;
    error: string | null;
  };
  let nativeUrl = $state(''),
    adoptId = $state(''),
    available = $state<{ id: string; name: string }[]>([]);
  type TransferReview = {
    review_id: string;
    name: string;
    image: string;
    source_config: string;
    managed_config: string;
    compose_project: string | null;
    compose_service: string | null;
  };
  let transferReview = $state<TransferReview | null>(null);
  let releasedCompose = $state(false);
  let releases = $state<
    { kind: string; image: string | null; tested_image: string }[]
  >([]);
  let services = $state<Service[]>([]),
    provisions = $state<Provision[]>([]),
    kind = $state('radarr'),
    hostPort = $state(17878),
    busy = $state(false),
    message = $state(''),
    loaded = $state(false);
  const ports: Record<string, number> = {
    radarr: 17878,
    sonarr: 18989,
    lidarr: 18686,
    bazarr: 16767,
    prowlarr: 19696,
    nzbget: 16789,
  };
  async function refresh() {
    const result = await api<{ items: Service[]; provisions: Provision[] }>(
      '/admin/stack',
    );
    services = result.items;
    provisions = result.provisions;
    const managers = await api<{ items: { id: string; name: string }[] }>(
      '/admin/managers',
    );
    const support = await api<{ items: { id: string; name: string }[] }>(
      '/admin/support',
    );
    available = [...managers.items, ...support.items].filter(
      (service) => !provisions.some((p) => p.service_id === service.id),
    );
    loaded = true;
  }
  async function act(work: () => Promise<void>) {
    busy = true;
    message = '';
    try {
      await work();
    } catch (error) {
      message = String(error);
    } finally {
      busy = false;
    }
  }
  onMount(() => {
    void act(refresh);
    const timer = setInterval(() => {
      if (
        !busy &&
        provisions.some((p) =>
          ['queued', 'installing', 'connecting'].includes(p.state),
        )
      )
        void act(refresh);
    }, 4000);
    return () => clearInterval(timer);
  });
</script>

<section class="panel">
  <h2>Managed services</h2>
  <p>
    Install optional services on this server. Choose library profiles and
    providers in their settings after installation. Advanced service UIs listen
    on the selected local port.
  </p>
  {#if message}<p role="status">{message}</p>{/if}
  <button class="secondary" disabled={busy} onclick={() => void act(refresh)}
    >Refresh managed services</button
  >
  {#if loaded}
    <button
      class="secondary"
      disabled={busy}
      onclick={() =>
        void act(async () => {
          releases = (
            await api<{ items: typeof releases }>('/admin/stack/releases')
          ).items;
        })}>Discover stable releases</button
    >
    {#if releases.length}<details open>
        <summary>Stable release discovery</summary
        >{#each releases as release (release.kind)}<p>
            {release.kind}: {release.image === release.tested_image
              ? 'Matches tested template'
              : release.image
                ? 'Requires compatibility verification'
                : 'Unable to verify'}{#if release.image}<br /><code
                >{release.image}</code
              >{/if}
          </p>{/each}
      </details>{/if}
    <button
      class="secondary"
      disabled={busy}
      onclick={() =>
        void act(async () => {
          await api('/admin/stack/wire', 'POST', {});
          await refresh();
        })}>Connect installed services</button
    >
    <form
      onsubmit={(event) => {
        event.preventDefault();
        void act(async () => {
          await api('/admin/stack/install', 'POST', {
            kind,
            host_port: hostPort,
            native_url: nativeUrl,
          });
          await refresh();
        });
      }}
    >
      <label
        >Service to install<select
          bind:value={kind}
          onchange={() => {
            hostPort = ports[kind];
          }}
          >{#each Object.keys(ports) as name (name)}<option value={name}
              >{name}</option
            >{/each}</select
        ></label
      >
      <label
        >Local service port<input
          type="number"
          min="1024"
          max="65535"
          bind:value={hostPort}
          required
        /></label
      >
      <label
        >Advanced UI address (optional)<input
          type="url"
          bind:value={nativeUrl}
          placeholder="https://radarr.example.test"
        /></label
      >
      <button
        class="secondary"
        disabled={busy || provisions.some((p) => p.kind === kind)}
        >Install service</button
      >
    </form>
    <form
      onsubmit={(event) => {
        event.preventDefault();
        void act(async () => {
          transferReview = await api<TransferReview>(
            '/admin/stack/adopt/preview',
            'POST',
            { service_id: adoptId },
          );
          releasedCompose = false;
        });
      }}
    >
      <label
        >Existing connected service<select
          bind:value={adoptId}
          required
          onchange={() => {
            transferReview = null;
            releasedCompose = false;
          }}
          ><option value="">Select a service</option
          >{#each available as service (service.id)}<option value={service.id}
              >{service.name}</option
            >{/each}</select
        ></label
      >
      <p>
        Take ownership of an existing service on this Docker server. Thelxinoe
        stops it, copies its configuration into managed storage and connects the
        replacement. Its settings, library and current version are preserved.
      </p>
      <button class="secondary" disabled={busy || !adoptId}
        >Review ownership transfer</button
      >
    </form>
    {#if transferReview}
      <section class="panel" aria-label="Ownership transfer review">
        <h3>Take ownership of {transferReview.name}</h3>
        <p>
          Copy configuration from <code>{transferReview.source_config}</code> to
          <code>{transferReview.managed_config}</code>.
        </p>
        <p>
          The service will be unavailable during the copy and restart. The
          original container stays stopped with automatic restart disabled, and
          its config folder is retained. The replacement keeps its API key, user
          and group, network aliases and published ports.
        </p>
        <details>
          <summary>Version retained during transfer</summary><code
            >{transferReview.image}</code
          >
        </details>
        {#if transferReview.compose_project}
          <p>
            In Compose project <strong>{transferReview.compose_project}</strong
            >, disable service <strong>{transferReview.compose_service}</strong> by
            removing its definition or assigning a profile you leave disabled. Save
            the file without redeploying it yet. Also disable any external updater
            for this service.
          </p>
          <Switch bind:checked={releasedCompose}
            >I disabled this service in its previous Compose project.</Switch
          >
        {:else}
          <p>
            Disable any script or external updater that recreates this container
            before transferring it.
          </p>
        {/if}
        <button
          class="secondary"
          disabled={busy ||
            (!!transferReview.compose_project && !releasedCompose)}
          onclick={() =>
            void act(async () => {
              await api('/admin/stack/adopt', 'POST', {
                service_id: adoptId,
                review_id: transferReview!.review_id,
                released_compose: releasedCompose,
              });
              transferReview = null;
              adoptId = '';
              await refresh();
            })}>Stop, copy and take ownership</button
        >
        <button
          class="secondary"
          disabled={busy}
          onclick={() => {
            transferReview = null;
          }}>Cancel</button
        >
      </section>
    {/if}
    {#each provisions as provision (provision.id)}<p>
        {provision.kind}: {provision.state}
        {#if provision.state === 'blocked'}<button
            class="secondary"
            disabled={busy}
            onclick={() =>
              void act(async () => {
                await api(`/admin/stack/${provision.id}/retry`, 'POST', {});
                await refresh();
              })}>Retry setup</button
          >
          {#if provision.origin === 'adopted' && !services.some((service) => service.id === provision.id && !service.transfer_pending)}
            <button
              class="secondary"
              disabled={busy}
              onclick={() =>
                void act(async () => {
                  await api(
                    `/admin/stack/${provision.id}/restore-original`,
                    'POST',
                    {},
                  );
                  await refresh();
                })}>Restore original service</button
            >
            <span
              >The original config will be used; changes made in the copied
              config are not merged.</span
            >
          {/if}
        {/if}
        {#if provision.native_url}<a
            href={provision.native_url}
            target="_blank"
            rel="noreferrer">Open service UI</a
          >{/if}{#if provision.error}
          · {provision.error}{/if}
      </p>{/each}
    {#each services as service (service.id)}
      <article class="panel">
        <h3 class="service-title">{service.kind}</h3>
        <p>
          {service.drift
            ? 'Configuration changed outside Thelxinoe'
            : service.running
              ? 'Running'
              : 'Stopped'} · {service.phase}
        </p>
        {#if service.error}<p>{service.error}</p>{/if}
        <details>
          <summary>Installed image</summary><code>{service.image}</code>
        </details>
        {#each ['start', 'stop', 'restart', 'reconcile'] as action (action)}<button
            class="secondary"
            disabled={busy ||
              provisions.some(
                (p) =>
                  p.id === service.id &&
                  ['queued', 'installing', 'connecting'].includes(p.state),
              ) ||
              (action !== 'reconcile' &&
                (service.drift || service.phase !== 'active'))}
            onclick={() =>
              void act(async () => {
                await api(`/admin/stack/${service.id}/action`, 'POST', {
                  action,
                });
                await refresh();
              })}>{action}</button
          >{/each}
      </article>
    {/each}
  {/if}
</section>

<style>
  form {
    display: grid;
    gap: 12px;
    margin: 20px 0;
  }
  p {
    margin: 10px 0;
    line-height: 1.5;
  }
  button {
    margin: 4px 8px 4px 0;
  }
  article {
    margin-top: 16px;
  }
  code {
    overflow-wrap: anywhere;
  }
  .service-title {
    text-transform: capitalize;
  }
</style>
