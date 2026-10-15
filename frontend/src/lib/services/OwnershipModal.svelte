<script lang="ts">
  import { onMount } from 'svelte';
  import {
    Check,
    CircleAlert,
    CircleDashed,
    CircleSlash,
    LoaderCircle,
    UserRound,
    Server,
  } from '@lucide/svelte';
  import { api, ApiError } from '../api';
  import Modal from '../ui/Modal.svelte';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import ProgressBar from '../ui/ProgressBar.svelte';
  import Switch from '../ui/Switch.svelte';
  import { stepHighlight } from '../ui/workflow-motion';
  import type {
    TransferDeclarations,
    TransferRecovery,
    TransferStep,
    TransferView,
  } from './feature';

  let {
    serviceId,
    transferId = null,
    label,
    onClose,
    changed,
    openService,
  } = $props<{
    serviceId: string;
    transferId?: string | null;
    label: string;
    onClose: () => void;
    changed?: () => Promise<void> | void;
    openService?: () => void;
  }>();

  let operation = $state<TransferView | null>(null);
  let loadError = $state('');
  let actionError = $state('');
  let pendingAction = $state('');
  const busy = $derived(!!pendingAction);
  let loadRequest = 0;
  let viewing = $state<string | null>(null);
  let stepContent = $state<HTMLElement>();
  let reviewed = $state(false);
  let recovery = $state<TransferRecovery | null>(null);
  let recoveryAction = $state('');
  const approvals = $state<Record<string, boolean>>({});
  const declarations = $state<TransferDeclarations>({
    cron: false,
    systemd: false,
    ci_cd: false,
    configuration_management: false,
    manual_compose: false,
    unobservable_automation: false,
    scoped_removed_from_files: false,
    scoped_no_orphan_removal: false,
    scoped_no_restart: false,
    scoped_no_appdata_writes: false,
    project_retired: false,
    unrelated_services_acknowledged: false,
  });

  const active = $derived(
    !!operation &&
      ['queued', 'running', 'committed', 'recovery'].includes(operation.state),
  );
  const plan = $derived(operation?.review.plan);
  const current = $derived(
    operation?.steps.find((step) => step.key === operation?.step) ??
      operation?.steps.find((step) =>
        ['running', 'awaiting_input', 'blocked', 'failed'].includes(
          step.status,
        ),
      ) ??
      operation?.steps.at(-1),
  );
  const shown = $derived(
    (viewing && operation?.steps.find((step) => step.key === viewing)) ||
      current,
  );
  const shownKey = $derived(shown?.key);
  const blockers = $derived(operation?.review.blockers ?? []);
  $effect(() => {
    if (shownKey) stepContent?.scrollTo({ top: 0, behavior: 'instant' });
  });
  const dockerAccess = $derived(
    plan?.automation.filter((item) => item.kind === 'docker_access') ?? [],
  );
  const declared = $derived(
    declarations.cron &&
      declarations.systemd &&
      declarations.ci_cd &&
      declarations.configuration_management &&
      declarations.manual_compose &&
      (!dockerAccess.length || declarations.unobservable_automation) &&
      (!plan?.compose ||
        (operation?.choices.compose_mode === 'scoped' &&
          declarations.scoped_removed_from_files &&
          declarations.scoped_no_orphan_removal &&
          declarations.scoped_no_restart &&
          declarations.scoped_no_appdata_writes) ||
        (operation?.choices.compose_mode === 'project' &&
          declarations.project_retired &&
          (!plan.compose.unrelated.length ||
            declarations.unrelated_services_acknowledged))),
  );

  function acceptOperation(next: TransferView) {
    if (
      operation &&
      (operation.id !== next.id ||
        operation.step !== next.step ||
        operation.state !== next.state)
    )
      viewing = null;
    operation = next;
    transferId = next.id;
  }
  async function load() {
    const request = ++loadRequest;
    try {
      const next = transferId
        ? await api<TransferView>(`/admin/transfers/${transferId}`)
        : await api<TransferView>(
            `/admin/services/${serviceId}/transfer`,
            'POST',
          );
      if (request !== loadRequest) return;
      acceptOperation(next);
      loadError = '';
    } catch (error) {
      if (request === loadRequest) loadError = String(error);
    }
  }
  async function act(action: string, work: () => Promise<void>) {
    if (busy) return;
    pendingAction = action;
    loadRequest++;
    actionError = '';
    try {
      await work();
      await changed?.();
    } catch (error) {
      actionError =
        error instanceof ApiError && error.code === 'revision_changed'
          ? `${error.message}. The review was refreshed; check it again.`
          : String(error);
      if (error instanceof ApiError && error.code === 'revision_changed')
        await load();
    } finally {
      pendingAction = '';
    }
  }
  function post(path: string, body: unknown = {}) {
    return api<TransferView>(
      `/admin/transfers/${operation!.id}${path}`,
      'POST',
      body,
    ).then((view) => {
      acceptOperation(view);
    });
  }
  async function reviewAgain() {
    reviewed = false;
    viewing = null;
    acceptOperation(
      await api<TransferView>(`/admin/services/${serviceId}/transfer`, 'POST'),
    );
  }
  async function chooseCompose(mode: 'scoped' | 'project') {
    reviewed = false;
    acceptOperation(
      await api<TransferView>(
        `/admin/transfers/${operation!.id}/choices`,
        'PUT',
        { compose_mode: mode },
      ),
    );
  }
  async function loadRecovery() {
    try {
      recovery = await api<TransferRecovery>(
        `/admin/transfers/${operation!.id}/recovery`,
      );
    } catch (error) {
      // The controller may already have reached an outcome; show it.
      await load();
      throw error;
    }
    recoveryAction = '';
    // Every acknowledgement starts unchecked and must be given for this plan.
    for (const option of recovery.options)
      for (const requirement of option.requires ?? [])
        approvals[requirement] = false;
  }

  onMount(() => {
    void load();
    let polling = false;
    // Durable state drives the modal; closing it never stops the work.
    const timer = setInterval(() => {
      if (active && !busy && !polling && !document.hidden) {
        polling = true;
        void load().finally(() => (polling = false));
      }
    }, 2000);
    return () => {
      loadRequest++;
      clearInterval(timer);
    };
  });

  function bytes(value?: number) {
    if (!value) return '';
    const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
    let size = value;
    let unit = 0;
    while (size >= 1024 && unit < units.length - 1) {
      size /= 1024;
      unit += 1;
    }
    return `${size.toFixed(unit ? 1 : 0)} ${units[unit]}`;
  }
  function shownValue(value: unknown) {
    if (value === null || value === undefined || value === '') return '—';
    return typeof value === 'string' ? value : JSON.stringify(value);
  }
  const statusText: Record<TransferStep['status'], string> = {
    complete: 'Done',
    running: 'In progress',
    awaiting_input: 'Your input',
    blocked: 'Blocked',
    failed: 'Failed',
    skipped: 'Skipped',
    pending: 'Upcoming',
  };
  const recoveryLabels: Record<string, string> = {
    repair: 'Repair and continue',
    restore_snapshot: 'Restore the transfer snapshot into a new candidate',
    return_original: 'Return to the original',
    suspend: 'Suspend for manual resolution',
  };
  const recoveryPhases: Record<string, string> = {
    accepted: 'The controller accepted the recovery',
    starting: 'Starting the replacement',
    'checking-original': 'Checking that the original stays stopped',
    'stopping-replacement': 'Stopping the replacement',
    'retiring-replacement':
      'Removing the stopped replacement and keeping its appdata',
    releasing: 'Releasing the replacement',
    'starting-original': 'Restarting the original',
    'removing-replacement': 'Removing the replacement',
    'preserving-appdata': 'Keeping the replaced appdata as a generation',
    building: 'Building a new candidate from the snapshot',
    creating: 'Creating the new candidate',
    activating: 'Starting the new candidate',
  };
  const explanations: Record<string, string> = {
    prerequisites:
      'Thelxinoe checked the connection, activity and deployment without changing the original.',
    review:
      'Review every change the transfer makes. Nothing changes until you confirm.',
    automation:
      'Thelxinoe can see only Docker. Confirm what it can’t observe so nothing restarts or rewrites the original.',
    confirm:
      'Confirm to stop the original and start the transfer. You can close this window afterwards; the work continues.',
    stop: 'Thelxinoe disables the original’s restart policy and stops it. Its container and appdata are kept.',
    copy: 'The stopped appdata is copied into an untouched snapshot and checked for completeness.',
    validate:
      'A separate copy runs in isolation with synthetic media and no network route to prove the state is preserved and readable.',
    activate:
      'The managed replacement is built from the snapshot plus your reviewed changes, created and started.',
    commit:
      'Thelxinoe and its controller record the replacement as an Owned service. Normal operations stay held.',
    setup: 'Only the setup actions you reviewed run against the replacement.',
    checks:
      'Required checks must pass before the transfer completes. Optional links are reported, not assumed.',
    complete:
      'Completion and the release of the operational hold happen together.',
  };
</script>

<Modal
  title={`Take ownership of ${label}`}
  size="workflow"
  {busy}
  onClose={() => {
    // Dismissing only closes the window; durable work and safety holds continue.
    onClose();
  }}
  closeLabel={active ? 'Continue in background' : 'Close'}
>
  {#if loadError}
    <Notice variant="error" role="alert"
      >{loadError}
      <Button
        size="sm"
        variant="secondary"
        loading={pendingAction === 'load'}
        onclick={() => void act('load', load)}>Retry</Button
      ></Notice
    >
  {:else if !operation || !plan}
    <div class="flex items-center gap-2 text-xs text-muted" role="status">
      <LoaderCircle
        size={14}
        class="animate-spin motion-reduce:animate-none"
        aria-hidden="true"
      />Reviewing {label} without changing it…
    </div>
  {:else}
    <div
      class="grid gap-5 @container/transfer grid-cols-[minmax(12rem,15rem)_minmax(0,1fr)] compact:flex compact:min-h-0 compact:flex-1 compact:flex-col compact:overflow-hidden"
    >
      <nav
        aria-label="Ownership transfer steps"
        class="relative isolate self-start compact:h-43 compact:w-full compact:shrink-0 compact:overflow-y-auto compact:overscroll-contain compact:scroll-smooth motion-reduce:scroll-auto compact:[scrollbar-gutter:stable]"
      >
        <div
          aria-hidden="true"
          data-transfer-highlight
          class="pointer-events-none absolute inset-x-0 top-0 border-l-2 border-accent bg-accent-soft"
          use:stepHighlight={shownKey}
        ></div>
        <ol class="relative grid gap-0.5 text-[11px]">
          {#each operation.steps as step, index (step.key)}
            <li>
              <button
                type="button"
                class="flex w-full items-center gap-2 border-l-2 px-2 py-1.5 text-left compact:min-h-14 data-[status=complete]:border-success data-[status=failed]:border-danger data-[status=blocked]:border-warning data-[status=running]:border-accent data-[status=awaiting_input]:border-accent data-[status=pending]:border-line data-[status=skipped]:border-line focus-visible:outline-2 focus-visible:outline-accent"
                data-transfer-step={step.key}
                data-status={step.status}
                aria-current={shown?.key === step.key ? 'step' : undefined}
                aria-label={`${index + 1}. ${step.label}: ${statusText[step.status]}, ${step.actor === 'user' ? 'your action' : 'done by Thelxinoe'}`}
                onclick={() => (viewing = step.key)}
              >
                {#if step.status === 'complete'}<Check
                    size={13}
                    class="shrink-0 text-success"
                    aria-hidden="true"
                  />{:else if step.status === 'running'}<LoaderCircle
                    size={13}
                    class="shrink-0 animate-spin text-accent motion-reduce:animate-none"
                    aria-hidden="true"
                  />{:else if step.status === 'failed' || step.status === 'blocked'}<CircleAlert
                    size={13}
                    class="shrink-0 {step.status === 'failed'
                      ? 'text-danger'
                      : 'text-warning'}"
                    aria-hidden="true"
                  />{:else if step.status === 'skipped'}<CircleSlash
                    size={13}
                    class="shrink-0 text-muted"
                    aria-hidden="true"
                  />{:else}<CircleDashed
                    size={13}
                    class="shrink-0 {step.status === 'awaiting_input'
                      ? 'text-accent'
                      : 'text-muted'}"
                    aria-hidden="true"
                  />{/if}
                <span class="min-w-0 flex-1">
                  <span
                    class="block {step.status === 'pending'
                      ? 'text-muted'
                      : ''}">{step.label}</span
                  >
                  <span
                    class="flex items-center gap-1 text-[9px] text-muted uppercase"
                    >{#if step.actor === 'user'}<UserRound
                        size={9}
                        aria-hidden="true"
                      />You{:else}<Server
                        size={9}
                        aria-hidden="true"
                      />Thelxinoe{/if} · {statusText[step.status]}</span
                  >
                </span>
              </button>
            </li>
          {/each}
        </ol>
      </nav>

      <section
        bind:this={stepContent}
        class="min-w-0 text-[11px] leading-[1.55] compact:min-h-0 compact:flex-1 compact:overflow-y-auto compact:overscroll-contain compact:[scrollbar-gutter:stable]"
        aria-live="polite"
        aria-label={shown?.label ?? 'Transfer'}
      >
        <h3 class="mb-1 text-[13px] font-[650]">{shown?.label}</h3>
        <p class="mb-3 text-muted">{explanations[shown?.key ?? ''] ?? ''}</p>
        {#if actionError}<Notice variant="error" role="alert"
            >{actionError}</Notice
          >{/if}

        {#if operation.state === 'complete'}
          <Notice tone="info" role="status">
            <strong>{label} is now Owned by Thelxinoe.</strong>
            <p>
              The managed replacement passed every required check and normal
              operations are released. The original container stays stopped with
              restart disabled, and its appdata at <code
                >{plan.source.appdata.source}</code
              > is preserved.
            </p>
          </Notice>
          {@render checksList(operation.checks)}
          <div class="mt-4 flex flex-wrap gap-2">
            <Button
              size="form"
              onclick={() => {
                openService?.();
                onClose();
              }}>Open {label}</Button
            >
            {#if operation.actions.remove_original}<Button
                size="form"
                variant="secondary"
                disabled={busy}
                loading={pendingAction === 'remove-original'}
                onclick={() =>
                  void act('remove-original', () => post('/original/remove'))}
                >Remove the stopped original container</Button
              >{/if}
          </div>
          {#if operation.actions.remove_original}<p
              class="mt-2 text-[10px] text-muted"
            >
              Removing the original deletes only its stopped container after
              verifying it is unchanged. Its appdata folder or volume is kept.
            </p>{/if}
        {:else if operation.state === 'review' && (shown?.key === 'prerequisites' || blockers.length)}
          {#if blockers.length}
            <Notice tone="warning" role="alert">
              <strong>Resolve these before the transfer can start</strong>
              <ul class="mt-1 list-disc pl-4">
                {#each blockers as blocker (blocker)}<li>{blocker}</li>{/each}
              </ul>
            </Notice>
            <Button
              size="form"
              variant="secondary"
              disabled={busy}
              loading={pendingAction === 'review'}
              onclick={() => void act('review', reviewAgain)}
              >Review again</Button
            >
          {:else}
            <p>
              The connection is verified, {label} is idle and its deployment is supported.
            </p>
            <Button size="form" onclick={() => (viewing = 'review')}
              >Continue to review</Button
            >
          {/if}
        {:else if operation.state === 'review' && shown?.key === 'review'}
          {@render reviewDetails()}
          <Button
            size="form"
            class="mt-4"
            onclick={() => (viewing = 'automation')}>Continue</Button
          >
        {:else if operation.state === 'review' && shown?.key === 'automation'}
          {@render automationStep()}
          <Button
            size="form"
            class="mt-4"
            disabled={!declared}
            onclick={() => (viewing = 'confirm')}>Continue</Button
          >
        {:else if operation.state === 'review'}
          <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5">
            <dt class="text-muted">Original</dt>
            <dd class="wrap-anywhere">
              {plan.source.name} stops and keeps its appdata ({plan.source
                .appdata.source})
            </dd>
            <dt class="text-muted">Replacement</dt>
            <dd class="wrap-anywhere">
              {plan.target.image} ({plan.target.version})
            </dd>
            <dt class="text-muted">Changes</dt>
            <dd>
              {plan.changes.length} deployment and {plan.settings.length} application
              changes
            </dd>
          </dl>
          <Switch class="mt-3" size="sm" bind:checked={reviewed}
            >I reviewed every change, and I understand the original stops</Switch
          >
          <div class="mt-4 flex flex-wrap gap-2">
            <Button
              size="form"
              disabled={busy || !reviewed || !declared || !!blockers.length}
              loading={pendingAction === 'approve'}
              onclick={() =>
                void act('approve', () =>
                  post('/approve', {
                    review_revision: operation!.review_revision,
                    confirm: true,
                    declarations,
                  }),
                )}>Take ownership</Button
            >
            <Button
              size="form"
              variant="secondary"
              disabled={busy}
              onclick={() => (viewing = 'review')}>Back to review</Button
            >
          </div>
        {:else if operation.state === 'recovery'}
          {@render recoveryProgress()}
        {:else if ['failed', 'suspended'].includes(operation.state)}
          {@render failure()}
        {:else if operation.state === 'returned' || operation.state === 'cancelled'}
          <Notice tone={operation.error ? 'warning' : 'info'} role="status">
            <strong
              >{operation.state === 'cancelled'
                ? 'The transfer was cancelled before anything changed.'
                : `${label} is Connected again.`}</strong
            >
            {#if operation.error}<p>{operation.error}</p>{/if}
            <p>
              The original container and its appdata are unchanged{operation.state ===
              'returned'
                ? ' and running as before'
                : ''}.
            </p>
          </Notice>
          <Button
            size="form"
            variant="secondary"
            disabled={busy}
            loading={pendingAction === 'review'}
            onclick={() => void act('review', reviewAgain)}>Review again</Button
          >
        {:else}
          {@render progressStep()}
        {/if}
      </section>
    </div>
  {/if}
</Modal>

{#snippet checksList(checks: TransferView['checks'])}
  {#if checks.length}
    <h4 class="mt-3 mb-1 text-[11px] font-[650]">Completion checks</h4>
    <ul class="grid gap-1">
      {#each checks as check (check.id)}
        <li class="flex items-start gap-2">
          <span
            class="mt-0.5 shrink-0 text-[9px] uppercase {check.status ===
            'passed'
              ? 'text-success'
              : check.status === 'failed'
                ? 'text-danger'
                : 'text-muted'}"
            >{check.status === 'passed'
              ? 'Passed'
              : check.status === 'failed'
                ? 'Failed'
                : check.required
                  ? 'Pending'
                  : 'Unavailable'}</span
          >
          <span
            >{check.label}<span class="text-muted">
              · {check.required ? 'Required' : 'Optional'}</span
            >{#if check.detail}<span class="block text-muted"
                >{check.detail}</span
              >{/if}</span
          >
        </li>
      {/each}
    </ul>
  {/if}
{/snippet}

{#snippet reviewDetails()}
  <div class="grid gap-4">
    <section aria-label="Containers">
      <h4 class="mb-1 font-[650]">Original and replacement</h4>
      <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1">
        <dt class="text-muted">Original container</dt>
        <dd class="wrap-anywhere">
          {plan!.source.name} · {plan!.source.image} ({plan!.source.version})
        </dd>
        <dt class="text-muted">Managed replacement</dt>
        <dd class="wrap-anywhere">
          {plan!.target.image} ({plan!.target
            .version}){#if plan!.target.decision.decision === 'upgrade'}<span
              class="block text-warning">{plan!.target.decision.reason}</span
            >{/if}
        </dd>
        <dt class="text-muted">Source appdata</dt>
        <dd class="wrap-anywhere">
          {plan!.source.appdata.kind}
          {plan!.source.appdata.source} at {plan!.source.appdata.destination}
          — preserved unchanged
        </dd>
        <dt class="text-muted">Managed appdata</dt>
        <dd class="wrap-anywhere">{plan!.target.appdata}</dd>
        <dt class="text-muted">Runtime identity</dt>
        <dd>
          {plan!.source.identity.uid}:{plan!.source.identity.gid} → {plan!
            .target.identity.uid}:{plan!.target.identity.gid}
        </dd>
      </dl>
    </section>
    <section aria-label="Deployment changes">
      <h4 class="mb-1 font-[650]">Deployment changes</h4>
      {@render changeTable(plan!.changes)}
    </section>
    {#if plan!.target.mounts.length}
      <section aria-label="Kept storage">
        <h4 class="mb-1 font-[650]">Media and other storage kept in place</h4>
        <ul class="grid gap-0.5">
          {#each plan!.target.mounts as mount (mount.destination)}<li
              class="wrap-anywhere"
            >
              {mount.destination} ← {mount.source}{mount.writable
                ? ''
                : ' (read-only)'}
            </li>{/each}
        </ul>
        <p class="text-muted">
          Media and download files are never copied, moved or re-owned.
        </p>
      </section>
    {/if}
    <section aria-label="Application changes">
      <h4 class="mb-1 font-[650]">Application settings changed in the copy</h4>
      {#if plan!.settings.length}{@render changeTable(plan!.settings)}{:else}<p
          class="text-muted"
        >
          None.
        </p>{/if}
    </section>
    {#if operation!.review.links.length}
      <section aria-label="Service links">
        <h4 class="mb-1 font-[650]">Service links</h4>
        <ul class="grid gap-0.5">
          {#each operation!.review.links as link (`${link.peer}-${link.direction}`)}<li
            >
              {link.peer} ({link.kind.replace('_', ' ')}): {link.action ===
              'stays_disconnected'
                ? 'stays disconnected, as you chose'
                : 'connects automatically after completion'}
            </li>{/each}
        </ul>
        <p class="text-muted">
          Existing matching records are offered for import after review instead
          of being overwritten.
        </p>
      </section>
    {/if}
    {#if operation!.review.dependencies.length}
      <section aria-label="External dependencies">
        <h4 class="mb-1 font-[650]">Dependencies that stay external</h4>
        <ul class="grid gap-0.5">
          {#each operation!.review.dependencies as dependency (dependency.host)}<li
            >
              {dependency.host}: {dependency.detail}
            </li>{/each}
        </ul>
      </section>
    {/if}
    {#if operation!.review.recyclarr_targets.length}
      <section aria-label="Recyclarr targets">
        <h4 class="mb-1 font-[650]">Recyclarr targets</h4>
        <ul class="grid gap-0.5">
          {#each operation!.review.recyclarr_targets as target (target.name)}<li
            >
              {target.name}: {shownValue(target.rewrite.before)} → {shownValue(
                target.rewrite.after,
              )}
            </li>{/each}
        </ul>
        <p class="text-muted">
          The imported schedule starts paused; synchronization runs only after
          you enable it.
        </p>
      </section>
    {/if}
    {#if operation!.review.setup_actions.length}
      <section aria-label="Setup actions">
        <h4 class="mb-1 font-[650]">Setup after the ownership commit</h4>
        <ul class="list-disc pl-4">
          {#each operation!.review.setup_actions as action (action.action)}<li>
              {action.label}
            </li>{/each}
        </ul>
      </section>
    {/if}
    {@render checksList(operation!.review.checks)}
  </div>
{/snippet}

{#snippet changeTable(changes: TransferView['review']['plan']['changes'])}
  <div class="overflow-x-auto">
    <table class="w-full border-collapse text-left">
      <thead class="text-[9px] text-muted uppercase"
        ><tr
          ><th class="py-1 pr-2 font-normal">Setting</th><th
            class="py-1 pr-2 font-normal">Before</th
          ><th class="py-1 pr-2 font-normal">After</th></tr
        ></thead
      >
      <tbody>
        {#each changes as change, index (index)}
          <tr class="border-t border-line align-top">
            <td class="py-1 pr-2">{change.setting ?? change.area}</td>
            <td class="py-1 pr-2 wrap-anywhere">{shownValue(change.before)}</td>
            <td class="py-1 pr-2 wrap-anywhere"
              >{shownValue(change.after)}<span class="block text-muted"
                >{change.reason}</span
              ></td
            >
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/snippet}

{#snippet automationStep()}
  <div class="grid gap-4">
    <section aria-label="Observed automation">
      <h4 class="mb-1 font-[650]">What Thelxinoe observed in Docker</h4>
      {#if plan!.automation.length}
        <ul class="grid gap-1">
          {#each plan!.automation as item (item.name)}<li>
              <strong>{item.name}</strong>
              ({item.kind === 'updater' ? 'updater' : 'Docker access'}): {item.scope}{#if item.blocks}<span
                  class="block text-danger"
                  >Its scope includes the original; exclude it first.</span
                >{/if}
            </li>{/each}
        </ul>
      {:else}<p class="text-muted">
          No updaters or other containers with Docker access.
        </p>{/if}
      {#if plan!.compose}
        <p class="mt-2">
          The original belongs to the Compose project <strong
            >{plan!.compose.project}</strong
          >{#if plan!.compose.config_files}
            ({plan!.compose.config_files}){/if}.
        </p>
      {/if}
    </section>
    {#if plan!.compose}
      <section aria-label="Compose ownership">
        <h4 class="mb-1 font-[650]">Compose project</h4>
        <div class="flex flex-wrap gap-2">
          <Button
            size="sm"
            variant={operation!.choices.compose_mode === 'scoped'
              ? 'default'
              : 'secondary'}
            aria-pressed={operation!.choices.compose_mode === 'scoped'}
            disabled={busy}
            loading={pendingAction === 'compose-scoped'}
            onclick={() =>
              void act('compose-scoped', () => chooseCompose('scoped'))}
            >Detach only this service</Button
          >
          <Button
            size="sm"
            variant={operation!.choices.compose_mode === 'project'
              ? 'default'
              : 'secondary'}
            aria-pressed={operation!.choices.compose_mode === 'project'}
            disabled={busy}
            loading={pendingAction === 'compose-project'}
            onclick={() =>
              void act('compose-project', () => chooseCompose('project'))}
            >Retire the whole project's automation</Button
          >
        </div>
        {#if operation!.choices.compose_mode === 'scoped'}
          <div class="mt-2 grid gap-1.5">
            <Switch
              size="sm"
              bind:checked={declarations.scoped_removed_from_files}
              >The service is removed from the project's Compose files</Switch
            >
            <Switch
              size="sm"
              bind:checked={declarations.scoped_no_orphan_removal}
              >Nothing runs the project with --remove-orphans, down or rm</Switch
            >
            <Switch size="sm" bind:checked={declarations.scoped_no_restart}
              >Nothing restarts or recreates the original</Switch
            >
            <Switch
              size="sm"
              bind:checked={declarations.scoped_no_appdata_writes}
              >Nothing writes the original's appdata</Switch
            >
          </div>
        {:else if operation!.choices.compose_mode === 'project'}
          <div class="mt-2 grid gap-1.5">
            {#if plan!.compose.unrelated.length}
              <p>
                These services stay running but lose automated deployment and
                updates through this project:
                {plan!.compose.unrelated
                  .map((service) => service.name)
                  .join(', ')}.
              </p>
              <Switch
                size="sm"
                bind:checked={declarations.unrelated_services_acknowledged}
                >I accept losing automated control of those services</Switch
              >
            {/if}
            <Switch size="sm" bind:checked={declarations.project_retired}
              >No automation or manual command will run against the project</Switch
            >
          </div>
        {/if}
      </section>
    {/if}
    <section aria-label="Administrator declarations">
      <h4 class="mb-1 font-[650]">What only you can confirm</h4>
      <p class="mb-2 text-muted">
        Thelxinoe can't inspect automation outside Docker and doesn't claim to
        have verified it. Your declarations are recorded separately from what it
        observed.
      </p>
      <div class="grid gap-1.5">
        <Switch size="sm" bind:checked={declarations.cron}
          >No off-host or host cron job changes the original or its copy</Switch
        >
        <Switch size="sm" bind:checked={declarations.systemd}
          >No systemd unit or timer restarts or recreates it</Switch
        >
        <Switch size="sm" bind:checked={declarations.ci_cd}
          >No CI/CD pipeline deploys it</Switch
        >
        <Switch size="sm" bind:checked={declarations.configuration_management}
          >No configuration-management tool (Ansible, Salt, Puppet, Chef)
          manages it</Switch
        >
        <Switch size="sm" bind:checked={declarations.manual_compose}
          >No one runs manual Compose commands that start or recreate it</Switch
        >
        {#if dockerAccess.length}<Switch
            size="sm"
            bind:checked={declarations.unobservable_automation}
            >{dockerAccess.map((item) => item.name).join(', ')} won't start, recreate
            or remove the original</Switch
          >{/if}
      </div>
    </section>
  </div>
{/snippet}

{#snippet progressStep()}
  {@const progress = operation!.progress}
  {#if progress?.waiting}
    <Notice tone="warning" role="status">{progress.waiting}</Notice>
  {:else}
    <ProgressBar
      class="mb-2 h-1"
      indeterminate
      label={`${shown?.label ?? 'Transfer'} in progress`}
    />
    {#if progress?.bytes || progress?.entries || progress?.files}<p
        class="text-muted"
      >
        {[
          progress.phase,
          progress.bytes ? `${bytes(progress.bytes)} processed` : '',
          progress.entries ? `${progress.entries} entries` : '',
          progress.files ? `${progress.files} files` : '',
        ]
          .filter(Boolean)
          .join(' · ')}
      </p>{:else if progress?.phase}<p class="text-muted">
        {progress.phase}
      </p>{/if}
  {/if}
  {#if operation!.checks.length}{@render checksList(operation!.checks)}{/if}
  <p class="mt-3 text-muted">
    You can continue in the background. Closing this window or leaving the page
    never cancels the transfer or releases its safety hold.
  </p>
  <div class="mt-3 flex flex-wrap gap-2">
    <Button size="form" variant="secondary" onclick={onClose}
      >Continue in background</Button
    >
    {#if operation!.actions.cancel}<Button
        size="form"
        variant="secondary"
        disabled={busy}
        loading={pendingAction === 'cancel'}
        onclick={() => void act('cancel', () => post('/cancel'))}
        >Cancel transfer (nothing has changed yet)</Button
      >{/if}
  </div>
{/snippet}

{#snippet recoveryProgress()}
  {@const command = operation!.recovery.command}
  {@const phase = operation!.controller?.command?.phase}
  <Notice tone="info" role="status">
    <strong
      >Applying the approved recovery: {recoveryLabels[command?.action ?? ''] ??
        'recovery'}</strong
    >
    <p>
      {recoveryPhases[phase ?? ''] ?? 'Waiting for the Docker controller'}.
      Thelxinoe continues this exact recovery from its journal after any
      interruption, and keeps normal operations held until it finishes.
    </p>
  </Notice>
  {@render progressStep()}
{/snippet}

{#snippet failure()}
  <Notice tone="warning" role="alert">
    <strong
      >{operation!.state === 'suspended'
        ? 'Transfer suspended for manual resolution'
        : `The transfer stopped at “${shown?.label}”`}</strong
    >
    {#if operation!.error}<p>{operation!.error}</p>{/if}
    <p>
      Completed: {operation!.steps
        .filter((step) => step.status === 'complete')
        .map((step) => step.label)
        .join(', ') || 'nothing'}.
      {operation!.controller?.boundary_at
        ? 'The replacement reached production, so Thelxinoe keeps normal operations held until you choose a recovery.'
        : 'The replacement never reached production.'}
    </p>
  </Notice>
  {#if operation!.checks.length}{@render checksList(operation!.checks)}{/if}
  <div class="mt-3 flex flex-wrap gap-2">
    {#if operation!.actions.retry}<Button
        size="form"
        disabled={busy}
        loading={pendingAction === 'retry'}
        onclick={() => void act('retry', () => post('/retry'))}
        >Retry setup and checks</Button
      >{/if}
    {#if operation!.actions.restore}<Button
        size="form"
        variant="secondary"
        disabled={busy}
        loading={pendingAction === 'restore'}
        onclick={() => void act('restore', () => post('/restore'))}
        >Restore original</Button
      >{/if}
    {#if operation!.actions.recovery && !recovery}<Button
        size="form"
        variant="secondary"
        disabled={busy}
        loading={pendingAction === 'recovery-review'}
        onclick={() => void act('recovery-review', loadRecovery)}
        >Review recovery options</Button
      >{/if}
  </div>
  {#if recovery}
    <section class="mt-4 grid gap-3" aria-label="Recovery plan">
      <div>
        <h4 class="mb-1 font-[650]">Evidence</h4>
        <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1">
          <dt class="text-muted">Production since</dt>
          <dd>
            {new Date(recovery.evidence.boundary_at * 1000).toLocaleString()}
          </dd>
          <dt class="text-muted">Replacement</dt>
          <dd>
            {recovery.evidence.containers.replacement.present
              ? recovery.evidence.containers.replacement.running
                ? 'Running'
                : 'Stopped'
              : 'Missing'} · {recovery.evidence.versions.replacement}
          </dd>
          <dt class="text-muted">Original</dt>
          <dd>
            {recovery.evidence.containers.original.present
              ? `${recovery.evidence.containers.original.running ? 'Running' : 'Stopped'}${recovery.evidence.containers.original.unchanged ? ', unchanged' : ', changed'}`
              : 'Missing'}
          </dd>
        </dl>
        {#if recovery.evidence.effects.known.length}<p class="mt-2">
            Observed since activation: {recovery.evidence.effects.known
              .map((effect) => `${effect.count} ${effect.effect}`)
              .join(', ')}.
          </p>{/if}
        {#each recovery.evidence.effects.uncertain as note (note)}<p
            class="text-muted"
          >
            {note}
          </p>{/each}
      </div>
      <Notice tone="warning">
        <strong>Recovery can't undo</strong>
        <ul class="mt-1 list-disc pl-4">
          {#each recovery.cannot_undo as item (item)}<li>{item}</li>{/each}
        </ul>
      </Notice>
      <fieldset class="grid gap-2">
        <legend class="mb-1 font-[650]">Choose a recovery</legend>
        {#each recovery.options as option (option.action)}
          <label
            class="flex items-start gap-2 {option.available
              ? ''
              : 'opacity-60'}"
          >
            <input
              type="radio"
              name="recovery"
              value={option.action}
              disabled={!option.available}
              bind:group={recoveryAction}
            />
            <span
              ><strong>{option.label}</strong><span class="block text-muted"
                >{option.description}</span
              >{#if !option.available && option.unavailable}<span
                  class="block text-muted"
                  >Unavailable: {option.unavailable}</span
                >{/if}</span
            >
          </label>
        {/each}
      </fieldset>
      {#each recovery.options.find((option) => option.action === recoveryAction)?.requires ?? [] as requirement (requirement)}
        <Switch size="sm" bind:checked={approvals[requirement]}
          >{requirement === 'state_loss'
            ? 'I accept that application state changed since activation may be lost'
            : requirement === 'reconciliation'
              ? 'I will reconcile peers, downloads and media listed above myself'
              : 'Automation around the original is still retired'}</Switch
        >
      {/each}
      <div>
        <Button
          size="form"
          loading={pendingAction === 'recovery-apply'}
          disabled={busy ||
            !recoveryAction ||
            (
              recovery.options.find(
                (option) => option.action === recoveryAction,
              )?.requires ?? []
            ).some((requirement) => !approvals[requirement])}
          onclick={() =>
            void act('recovery-apply', async () => {
              await post('/recovery', {
                action: recoveryAction,
                revision: recovery!.revision,
                approval: { ...approvals },
              });
              recovery = null;
            })}>Apply recovery</Button
        >
      </div>
    </section>
  {/if}
{/snippet}
