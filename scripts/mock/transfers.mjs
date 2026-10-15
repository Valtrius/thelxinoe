import { label, now } from './data.mjs';
import { serverActionDelay } from './timing.mjs';

// Preview-only model of the two service stages and the guided ownership transfer.
const steps = [
  ['prerequisites', 'Check prerequisites', 'backend'],
  ['review', 'Review changes', 'user'],
  ['automation', 'Confirm automation is retired', 'user'],
  ['confirm', 'Confirm the transfer', 'user'],
  ['stop', 'Stop the original', 'backend'],
  ['copy', 'Copy appdata', 'backend'],
  ['validate', 'Validate a disposable copy', 'backend'],
  ['activate', 'Start the managed replacement', 'backend'],
  ['commit', 'Commit ownership', 'backend'],
  ['setup', 'Run reviewed setup', 'backend'],
  ['checks', 'Verify required checks', 'backend'],
  ['complete', 'Complete the transfer', 'backend'],
];
const running = steps.slice(4).map(([key]) => key);
const checks = (kind) =>
  [
    ['docker_storage', 'Accepted Docker and storage identity', true],
    ['preserved_state', 'Preserved application state', true],
    ['state_format', 'Supported persisted-state format', true],
    ['runtime_access', 'Permitted runtime and media access', true],
    ['api_ready', 'Authenticated API readiness', true],
    ...(['radarr', 'sonarr', 'lidarr'].includes(kind)
      ? [
          ['library', 'Preserved library identities', true],
          ['files', 'Populated roots and files resolve', true],
          ['url_base', 'Managed URL Base', true],
          ['nzbget_link', 'NZBGet download client link', false],
          ['prowlarr_link', 'Prowlarr application link', false],
        ]
      : [['links', 'Service links', false]]),
  ].map(([id, text, required]) => ({
    id,
    label: text,
    required,
    status: 'pending',
    detail: null,
    evidence: null,
  }));

function review(kind, service) {
  return {
    plan: {
      review_id: `transfer-${kind}`,
      revision: `revision-${kind}`,
      kind,
      source: {
        container_id: service.container_id,
        name: kind,
        image: `lscr.io/linuxserver/${kind}@sha256:preview`,
        version: service.version ?? '1.0.0',
        running: true,
        restart_policy: 'unless-stopped',
        networks: ['media'],
        appdata: {
          kind: 'bind',
          source: `/srv/appdata/${kind}`,
          destination: '/config',
          writable: true,
          role: 'appdata',
        },
        identity: { uid: 1000, gid: 1000 },
      },
      target: {
        image: `lscr.io/linuxserver/${kind}@sha256:preview`,
        version: service.version ?? '1.0.0',
        decision: {
          decision: 'same',
          image: `lscr.io/linuxserver/${kind}@sha256:preview`,
        },
        appdata: `Thelxinoe deployment storage (services/transfer-${kind}/appdata)`,
        appdata_destination: '/config',
        identity: { uid: 10001, gid: 10001 },
        aliases: [kind, `thelxinoe-${kind}`],
        published_port: null,
        mounts: [
          {
            kind: 'bind',
            source: '/srv/media',
            destination: '/media',
            writable: true,
            role: 'media',
          },
        ],
        supplementary: [],
      },
      changes: [
        {
          area: 'ports',
          before: `0.0.0.0:${service.port} → ${service.port}/tcp`,
          after: 'Not published',
          reason:
            'Managed services publish only on this host’s loopback address',
        },
        {
          area: 'identity',
          before: '1000:1000',
          after: '10001:10001',
          reason:
            'The copied configuration is owned by Thelxinoe’s media identity',
        },
      ],
      settings: [
        {
          setting: 'URL Base',
          before: '',
          after: `/services/${kind}`,
          reason: 'Thelxinoe opens the service at its managed route',
        },
      ],
      automation: [
        {
          kind: 'docker_access',
          name: 'portainer',
          scope:
            'Unknown: this container controls Docker and Thelxinoe can’t inspect what it automates',
          blocks: false,
        },
      ],
      compose: {
        project: 'media',
        service: kind,
        working_dir: '/srv/compose',
        config_files: '/srv/compose/compose.yaml',
        unrelated: [{ name: 'jellyfin', service: 'jellyfin', running: true }],
      },
      blockers: [],
    },
    blockers: [],
    links: [],
    dependencies: [],
    checks: checks(kind),
    setup_actions: [],
    recyclarr_targets: [],
  };
}

function own(s, t) {
  let owned = s.stack.find((item) => item.kind === t.kind);
  if (!owned) {
    owned = {
      id: t.id,
      kind: t.kind,
      name: label(t.kind),
      registered: true,
      phase: 'active',
      image: t.review.plan.target.image,
      drift: false,
      running: true,
      existence: 'present',
      status: 'running',
      inspection_error: null,
      can_recreate: true,
      can_retire: true,
      can_remove: true,
      error: null,
      transfer_pending: true,
      provision: {
        id: t.id,
        kind: t.kind,
        state: 'complete',
        host_port: null,
        container_id: `replacement-${t.kind}`,
        service_id: t.service_id,
        error: null,
        origin: 'transferred',
        transfer_id: t.id,
      },
    };
    s.stack.push(owned);
  }
  owned.provision.hold = t.state === 'complete' ? null : 'transfer';
  owned.transfer_pending = t.state !== 'complete';
  const service = s.services.find((item) => item.id === t.service_id);
  if (service) service.container_id = owned.provision.container_id;
  t.replacement_id = owned.provision.container_id;
}

function returnOriginal(s, t) {
  s.stack = s.stack.filter((item) => item.kind !== t.kind);
  const service = s.services.find((item) => item.id === t.service_id);
  if (service) {
    service.container_id = t.review.plan.source.container_id;
    service.error = null;
  }
  t.state = 'returned';
  t.error = null;
  t.progress = null;
}

export function createTransfer(s, service) {
  s.transfers ??= {};
  const t = {
    id: `transfer-${service.kind}`,
    kind: service.kind,
    service_id: service.id,
    state: 'review',
    step: 'review',
    review: review(service.kind, service),
    review_revision: `revision-${service.kind}`,
    choices: {},
    recovery: {},
    progress: null,
    error: null,
    replacement_id: null,
    updated: Date.now(),
    setup: [],
  };
  t.review.links = s.connections
    .filter(
      (item) => item.source_id === service.id || item.target_id === service.id,
    )
    .map((item) => ({
      kind: item.kind,
      peer: label(
        item.source_id === service.id ? item.target_kind : item.source_kind,
      ),
      peer_kind:
        item.source_id === service.id ? item.target_kind : item.source_kind,
      direction: item.source_id === service.id ? 'outgoing' : 'incoming',
      action: 'connect_after_completion',
    }));
  s.transfers[t.id] = t;
  return t;
}

export function setTransferStep(s, t, step) {
  t.step = step;
  t.state = running.indexOf(step) >= 4 ? 'committed' : 'running';
  t.progress =
    step === 'copy'
      ? { phase: 'copying', bytes: 48 * 1024 ** 2, entries: 310 }
      : { phase: step };
  if (running.indexOf(step) >= 3) t.boundary_at ??= now();
  if (running.indexOf(step) >= 4) own(s, t);
  if (step === 'complete') {
    t.state = 'complete';
    t.progress = null;
    own(s, t);
  }
  const outcome = s.scenarios?.services[t.kind]?.outcome;
  if (
    (outcome === 'before' && step === 'validate') ||
    (['after', 'unavailable'].includes(outcome) && step === 'checks')
  ) {
    t.state = 'failed';
    t.error =
      step === 'validate'
        ? 'The disposable copy failed validation. The original is preserved.'
        : 'Required check failed: Authenticated API readiness';
  }
  t.updated = Date.now();
}

export function advanceTransfer(s, t) {
  if (t.state === 'recovery') {
    const action = t.recovery.command.action;
    t.recovery.command.outcome = 'applied';
    if (action === 'return_original') returnOriginal(s, t);
    else if (action === 'suspend') t.state = 'suspended';
    else {
      t.error = null;
      setTransferStep(s, t, 'setup');
    }
    t.updated = Date.now();
  } else if (['queued', 'running', 'committed'].includes(t.state)) {
    setTransferStep(
      s,
      t,
      running[Math.min(running.indexOf(t.step) + 1, running.length - 1)],
    );
  }
}

export function syncTransfers(s) {
  for (const t of Object.values(s.transfers ?? {})) {
    if (s.scenarios?.services[t.kind]?.paused) continue;
    let elapsed = Math.min(
      8,
      Math.floor((Date.now() - t.updated) / serverActionDelay),
    );
    while (
      elapsed-- > 0 &&
      ['queued', 'running', 'committed', 'recovery'].includes(t.state)
    )
      advanceTransfer(s, t);
  }
}

export function transferView(t) {
  const current = steps.findIndex(([key]) => key === t.step);
  return {
    ...t,
    updated: undefined,
    steps: steps.map(([key, text, actor], index) => ({
      key,
      label: text,
      actor,
      status:
        t.state === 'complete'
          ? 'complete'
          : t.state === 'review'
            ? index === 0
              ? t.review.blockers.length
                ? 'blocked'
                : 'complete'
              : index < 4
                ? 'awaiting_input'
                : 'pending'
            : index < current
              ? 'complete'
              : index === current
                ? ['failed', 'suspended'].includes(t.state)
                  ? 'failed'
                  : 'running'
                : 'pending',
    })),
    checks:
      t.state === 'complete' || t.step === 'checks'
        ? t.review.checks.map((check) => ({
            ...check,
            status:
              t.state === 'failed' && check.id === 'api_ready'
                ? 'failed'
                : check.required
                  ? 'passed'
                  : 'pending',
            detail: check.required
              ? null
              : 'Connects automatically after completion when both services are ready',
          }))
        : [],
    actions: {
      approve: t.state === 'review' && !t.review.blockers.length,
      cancel: t.state === 'review' || t.state === 'queued',
      retry: ['failed', 'suspended'].includes(t.state),
      restore: t.state === 'failed' && !t.boundary_at,
      recovery: ['failed', 'suspended'].includes(t.state) && !!t.boundary_at,
      remove_original: t.state === 'complete' && !t.original_removed,
      continue_in_background: [
        'queued',
        'running',
        'committed',
        'recovery',
      ].includes(t.state),
    },
    controller:
      t.state === 'review'
        ? null
        : {
            stage: t.state,
            boundary_at: t.boundary_at ?? null,
            completed: t.state === 'complete',
            source: {
              stopped: current >= 4 && t.state !== 'returned',
              restart_disabled: current >= 4 && t.state !== 'returned',
              removed: !!t.original_removed,
            },
            command:
              t.state === 'recovery'
                ? {
                    operation: `recovery-${t.id}`,
                    phase:
                      t.recovery.command.action === 'return_original'
                        ? 'starting-original'
                        : 'starting',
                    state: 'running',
                  }
                : null,
          },
  };
}

export async function connectionFailure(s, kind) {
  const config = s.scenarios?.services[kind];
  const result = config?.result;
  return result === 'credentials'
    ? 'Invalid API key or login credentials.'
    : result === 'unreachable'
      ? 'The service could not be reached.'
      : null;
}

export async function checkConnection(s, c, service, save = true) {
  const error = await connectionFailure(s, service.kind);
  if (save) {
    service.error = error;
    service.checked_at = now();
  }
  c.emit('services.changed', {});
  return error
    ? c.fail(422, error)
    : c.json({ healthy: true, version: service.version });
}

function recoveryPlan(s, t) {
  const missing = s.scenarios?.services[t.kind]?.outcome === 'unavailable';
  return {
    revision: `recovery-${t.id}`,
    evidence: {
      boundary_at: t.boundary_at,
      images: {
        source: t.review.plan.source.image,
        replacement: t.review.plan.target.image,
      },
      versions: {
        source: t.review.plan.source.version,
        replacement: t.review.plan.target.version,
      },
      containers: {
        replacement: { present: true, running: true },
        original: { present: !missing, running: false, unchanged: !missing },
      },
      effects: {
        known: [{ effect: 'application settings changed', count: 1 }],
        uncertain: ['Downloads grabbed after activation are not tracked'],
      },
    },
    options: [
      {
        action: 'repair',
        label: 'Repair and continue',
        available: !missing,
        unavailable: missing
          ? 'The original container is missing; its state cannot be verified'
          : null,
        description: 'Repair the replacement and run setup and checks again',
        requires: ['automation'],
      },
      {
        action: 'restore_snapshot',
        label: 'Restore the transfer snapshot',
        available: !missing,
        unavailable: missing
          ? 'The original container is missing; recovery evidence is incomplete'
          : null,
        description:
          'Create a new candidate from the preserved transfer snapshot',
        requires: ['state_loss', 'reconciliation'],
      },
      {
        action: 'return_original',
        label: 'Return to the original',
        available: !missing,
        unavailable: missing ? 'The original container is missing' : null,
        description: 'Stop the replacement and start the unchanged original',
        requires: ['state_loss', 'reconciliation'],
      },
      {
        action: 'suspend',
        label: 'Suspend for manual resolution',
        available: true,
        description: 'Keep normal operations held for manual resolution',
      },
    ],
    cannot_undo: ['Indexer searches already sent'],
    transfer_written: [],
    peer_note: '',
  };
}

export async function stagesApi(s, c) {
  const { path, method, body, json, fail } = c;
  s.transfers ??= {};
  syncTransfers(s);
  if (path === '/admin/services' && method === 'GET') {
    return json({
      items: [
        'seerr',
        'recyclarr',
        'radarr',
        'sonarr',
        'lidarr',
        'bazarr',
        'prowlarr',
        'nzbget',
      ].map((kind) => {
        const service = s.services.find((item) => item.kind === kind);
        const owned = s.stack.find((item) => item.kind === kind);
        const transfer = Object.values(s.transfers).find(
          (item) =>
            item.kind === kind &&
            !['review', 'complete', 'cancelled', 'returned'].includes(
              item.state,
            ),
        );
        const draft = Object.values(s.transfers).find(
          (item) => item.kind === kind && item.state === 'review',
        );
        const hold = owned && transfer ? 'transfer' : null;
        return {
          kind,
          stage: owned ? 'owned' : service ? 'connected' : 'none',
          hold,
          agreed: true,
          draft_review: draft?.id ?? null,
          actions: [],
          connection: service
            ? {
                id: service.id,
                container_id: `${service.container_id}`.padEnd(64, '0'),
                container_name: kind,
                engine: 'preview-engine',
                version: service.version ?? '',
                verified: !service.error,
                operational: !!owned && !transfer,
                checked_at: service.checked_at ?? now(),
                error: service.error ?? null,
              }
            : null,
          ownership: owned ? { ...owned.provision, hold } : null,
          operation: transfer
            ? {
                type: 'transfer',
                id: transfer.id,
                state: transfer.state,
                step: transfer.step,
                error: transfer.error,
              }
            : null,
        };
      }),
    });
  }
  const serviceAction = path.match(
    /^\/admin\/services\/([^/]+)\/(transfer|test|forget)$/,
  );
  if (serviceAction && method === 'POST') {
    const [, id, action] = serviceAction;
    const service = s.services.find((item) => item.id === id);
    if (!service) return fail(404, 'Service not found');
    if (action === 'test') return checkConnection(s, c, service);
    if (action === 'forget') {
      s.services = s.services.filter((item) => item !== service);
      return json({ forgotten: true });
    }
    const existing = Object.values(s.transfers).find(
      (item) =>
        item.kind === service.kind &&
        !['complete', 'returned', 'cancelled'].includes(item.state),
    );
    return json(transferView(existing ?? createTransfer(s, service)));
  }
  if (path === '/admin/transfers' && method === 'GET')
    return json({ items: Object.values(s.transfers).map(transferView) });
  const transfer = path.match(/^\/admin\/transfers\/([^/]+)(?:\/(.*))?$/);
  if (transfer) {
    const t = s.transfers[transfer[1]];
    if (!t) return fail(404, 'Transfer not found');
    const action = transfer[2];
    if (!action && method === 'GET') return json(transferView(t));
    if (action === 'choices' && t.state === 'review') {
      t.choices = { compose_mode: body.compose_mode };
      t.review_revision = `revision-${t.kind}-${body.compose_mode}`;
    } else if (action === 'approve' && t.state === 'review') {
      if (body.review_revision !== t.review_revision)
        return fail(
          409,
          'The review changed; check it again before confirming',
        );
      if (!body.confirm || t.review.blockers.length)
        return fail(409, 'Review the transfer prerequisites first');
      setTransferStep(s, t, 'stop');
    } else if (action === 'cancel' && ['review', 'queued'].includes(t.state))
      t.state = 'cancelled';
    else if (action === 'original/remove' && t.state === 'complete')
      t.original_removed = true;
    else if (action === 'retry' && ['failed', 'suspended'].includes(t.state)) {
      if (s.scenarios?.services[t.kind])
        s.scenarios.services[t.kind].outcome = 'success';
      t.error = null;
      setTransferStep(s, t, t.boundary_at ? 'setup' : 'validate');
    } else if (action === 'restore' && t.state === 'failed' && !t.boundary_at)
      returnOriginal(s, t);
    else if (
      action === 'recovery' &&
      ['failed', 'suspended'].includes(t.state) &&
      t.boundary_at
    ) {
      const plan = recoveryPlan(s, t);
      if (method === 'GET') return json(plan);
      const option = plan.options.find((item) => item.action === body.action);
      if (
        body.revision !== plan.revision ||
        !option?.available ||
        option.requires?.some((key) => !body.approval?.[key])
      )
        return fail(
          409,
          'Recovery requires a current plan and every acknowledgement',
        );
      t.state = 'recovery';
      t.recovery = {
        command: { operation: `recovery-${t.id}`, action: body.action },
      };
      t.updated = Date.now();
      if (s.scenarios?.services[t.kind] && body.action !== 'suspend')
        s.scenarios.services[t.kind].outcome = 'success';
    } else
      return fail(
        409,
        'This action is not available at the current transfer step',
      );
    c.emit('services.changed', {});
    return json(transferView(t));
  }
  if (path === '/admin/recyclarr/connection' && method === 'POST')
    return json({ id: 'connected-recyclarr', healthy: true, version: '8.7.2' });
  return false;
}
