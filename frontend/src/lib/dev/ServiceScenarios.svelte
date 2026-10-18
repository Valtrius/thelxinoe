<script lang="ts">
  import { onMount } from 'svelte';
  import { FlaskConical, X } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import ExclusiveChoiceGroup from '../ui/ExclusiveChoiceGroup.svelte';
  import FormField from '../ui/FormField.svelte';
  import { formControlClass } from '../ui/styles';

  type Controls = {
    preset: string;
    result: string;
    delay: number;
    linkState: string;
    outcome: string;
    paused: boolean;
  };
  type Viewer = 'admin' | 'user';
  type Preview = {
    kinds: { kind: string; label: string }[];
    presets: { value: string; label: string }[];
    services: Record<string, Controls>;
    transfers: Record<
      string,
      {
        id: string;
        service_id: string;
        kind: string;
        state: string;
        step: string;
      }
    >;
  };
  let open = $state(sessionStorage.getItem('mock-services-open') === 'true');
  let kind = $state(sessionStorage.getItem('mock-service') ?? 'prowlarr');
  let preview = $state<Preview | null>(null);
  let values = $state<Controls>({
    preset: 'owned',
    result: 'success',
    delay: 3000,
    linkState: 'connected',
    outcome: 'success',
    paused: false,
  });
  let viewer = $state<Viewer | null>(null);
  let busy = $state(false);
  let error = $state('');
  let status = $state('');
  const transfer = $derived(preview?.transfers[kind]);

  function toggle() {
    open = !open;
    sessionStorage.setItem('mock-services-open', String(open));
  }
  async function read() {
    const response = await fetch('/__mock/services');
    if (!response.ok) throw Error('Could not load service scenarios');
    preview = await response.json();
  }
  function selectService() {
    sessionStorage.setItem('mock-service', kind);
    values = { ...preview!.services[kind] };
    status = '';
  }
  async function act(action: string) {
    busy = true;
    error = '';
    status = '';
    try {
      const response = await fetch('/__mock/services', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action, kind, ...values }),
      });
      if (!response.ok) throw Error((await response.json()).error);
      preview = await response.json();
      values = { ...preview!.services[kind] };
      if (action === 'scenario' || action === 'reset') {
        location.hash = `admin/services/${kind}`;
        location.reload();
      } else {
        status = action === 'advance' ? 'Step advanced.' : 'Controls applied.';
        document.dispatchEvent(new Event('visibilitychange'));
        window.dispatchEvent(new Event('thelxinoe-attention-refresh'));
      }
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }
  async function view(role: Viewer) {
    busy = true;
    error = '';
    try {
      const response = await fetch('/__mock/viewer', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ role }),
      });
      if (!response.ok) throw Error((await response.json()).error);
      location.reload();
    } catch (failure) {
      error = String(failure);
      open = true;
      busy = false;
    }
  }
  onMount(() => {
    void fetch('/__mock/viewer')
      .then((response) => response.json())
      .then((value) => (viewer = value.role))
      .catch((failure) => (error = String(failure)));
    void read()
      .then(() => selectService())
      .catch((failure) => (error = String(failure)));
    const timer = setInterval(() => {
      if (open) void read().catch((failure) => (error = String(failure)));
    }, 1500);
    return () => clearInterval(timer);
  });
</script>

<div
  class="fixed bottom-3 right-3 z-[200] max-w-[calc(100vw-1.5rem)] text-foreground"
>
  {#if open}
    <section
      aria-label="Service scenarios"
      class="mb-2 max-h-[min(78dvh,44rem)] w-80 max-w-full overflow-y-auto border border-line-strong bg-surface-strong p-4 shadow-panel text-xs"
    >
      <div class="mb-3 flex items-center justify-between gap-3">
        <h2 class="flex items-center gap-2 text-sm font-semibold">
          <FlaskConical size={15} class="text-accent" />Service scenarios
        </h2>
        <Button
          variant="ghost"
          size="compact-icon"
          aria-label="Close service scenarios"
          onclick={toggle}><X size={14} /></Button
        >
      </div>
      {#if preview}
        <div class="grid gap-3 [&_label]:m-0">
          <FormField
            >Preview service<select
              aria-label="Preview service"
              class={formControlClass}
              bind:value={kind}
              onchange={selectService}
              disabled={busy}
              >{#each preview.kinds as item (item.kind)}<option
                  value={item.kind}>{item.label}</option
                >{/each}</select
            ></FormField
          >
          <div class="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2">
            <FormField
              >Scenario<select
                aria-label="Scenario"
                class={formControlClass}
                bind:value={values.preset}
                disabled={busy}
                >{#each preview.presets as item (item.value)}<option
                    value={item.value}>{item.label}</option
                  >{/each}</select
              ></FormField
            >
            <Button
              size="form"
              disabled={busy}
              onclick={() => void act('scenario')}>Load scenario</Button
            >
          </div>
          <div class="grid grid-cols-2 gap-2 border-t border-line pt-3">
            <FormField
              >Connection result<select
                aria-label="Connection result"
                class={formControlClass}
                bind:value={values.result}
                disabled={busy}
                ><option value="success">Success</option><option
                  value="credentials">Invalid credentials</option
                ><option value="unreachable">Unreachable</option></select
              ></FormField
            >
            <FormField
              >Response delay<select
                aria-label="Response delay"
                class={formControlClass}
                bind:value={values.delay}
                disabled={busy}
                ><option value={0}>None</option><option value={500}
                  >500 ms</option
                ><option value={1500}>1.5 seconds</option><option value={3000}
                  >3 seconds</option
                ><option value={5000}>5 seconds</option></select
              ></FormField
            >
          </div>
          <FormField
            >Service link state<select
              aria-label="Service link state"
              class={formControlClass}
              bind:value={values.linkState}
              disabled={busy}
              ><option value="connected">Connected</option><option
                value="available">Available</option
              ><option value="pending">Connecting</option><option
                value="unavailable">Unavailable</option
              ><option value="conflict">Existing record conflict</option><option
                value="cleanup">Disconnect pending</option
              ></select
            ></FormField
          >
          <FormField
            >Transfer outcome<select
              aria-label="Transfer outcome"
              class={formControlClass}
              bind:value={values.outcome}
              disabled={busy}
              ><option value="success">Success</option><option value="before"
                >Fail before activation</option
              ><option value="after">Fail after activation</option><option
                value="unavailable">Fail, original missing</option
              ></select
            ></FormField
          >
          <label class="flex items-center gap-2"
            ><input
              type="checkbox"
              class="accent-accent"
              bind:checked={values.paused}
              disabled={busy}
            />Pause transfer progress</label
          >
          <div class="flex flex-wrap gap-2">
            <Button
              size="form"
              disabled={busy}
              onclick={() => void act('settings')}>Apply controls</Button
            >
            <Button
              size="form"
              variant="secondary"
              disabled={busy ||
                !transfer ||
                !['queued', 'running', 'committed', 'recovery'].includes(
                  transfer.state,
                )}
              onclick={() => void act('advance')}>Advance step</Button
            >
            {#if transfer}<Button
                size="form"
                variant="secondary"
                disabled={busy}
                onclick={() => {
                  open = false;
                  sessionStorage.setItem('mock-services-open', 'false');
                  window.dispatchEvent(
                    new CustomEvent('thelxinoe-mock-transfer', {
                      detail: transfer,
                    }),
                  );
                }}>Inspect transfer</Button
              >{/if}
          </div>
          {#if transfer}<p class="text-muted" role="status">
              {transfer.state} · {transfer.step}
            </p>{/if}
          <div class="border-t border-line pt-3">
            <Button
              variant="secondary"
              size="sm"
              disabled={busy}
              onclick={() => void act('reset')}>Reset all services</Button
            >
          </div>
        </div>
      {/if}
      {#if error}<p class="mt-2 text-danger" role="alert">{error}</p>{/if}
      {#if status}<p class="mt-2 text-muted" role="status">{status}</p>{/if}
    </section>
  {/if}
  <div class="flex items-center justify-end gap-2">
    {#if viewer}<div class="bg-surface-strong">
        <ExclusiveChoiceGroup
          ariaLabel="Preview view"
          choices={[
            { value: 'admin', label: 'Admin' },
            { value: 'user', label: 'User' },
          ]}
          value={viewer}
          disabled={busy}
          onChange={(role) => void view(role)}
        />
      </div>{/if}
    <Button
      size="sm"
      variant="secondary"
      class="bg-surface-strong"
      aria-expanded={open}
      onclick={toggle}><FlaskConical size={13} />Service scenarios</Button
    >
  </div>
</div>
