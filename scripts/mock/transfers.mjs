import { label, now } from './data.mjs';

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
          before: '0.0.0.0:7878 → 7878/tcp',
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

function view(t) {
  const elapsed = Math.floor((Date.now() - (t.started ?? Date.now())) / 1500);
  if (['queued', 'running', 'committed'].includes(t.state)) {
    const index = Math.min(elapsed, running.length);
    if (index >= running.length) {
      t.state = 'complete';
      t.step = 'complete';
      t.complete?.();
    } else {
      t.step = running[index];
      t.state = index >= 4 ? 'committed' : 'running';
      t.progress =
        t.step === 'copy'
          ? {
              phase: 'copying',
              bytes: 48_000_000 * (elapsed + 1),
              entries: 310,
            }
          : { phase: t.step };
    }
  }
  const current = steps.findIndex(([key]) => key === t.step);
  return {
    ...t,
    started: undefined,
    complete: undefined,
    steps: steps.map(([key, text, actor], index) => ({
      key,
      label: text,
      actor,
      // Services without reviewed setup actions skip that step.
      status:
        key === 'setup' && !t.review.setup_actions.length
          ? 'skipped'
          : t.state === 'complete'
            ? 'complete'
            : t.state === 'review'
              ? index === 0
                ? 'complete'
                : index < 4
                  ? 'awaiting_input'
                  : 'pending'
              : index < current
                ? 'complete'
                : index === current
                  ? 'running'
                  : 'pending',
    })),
    checks:
      t.state === 'complete'
        ? t.review.checks.map((check) => ({
            ...check,
            status: check.required ? 'passed' : 'pending',
            detail: check.required
              ? null
              : 'Connects automatically after completion when both services are ready',
          }))
        : [],
    actions: {
      approve: t.state === 'review',
      cancel: t.state === 'review' || t.state === 'queued',
      retry: false,
      restore: false,
      recovery: false,
      remove_original: t.state === 'complete' && !t.original_removed,
      continue_in_background: ['queued', 'running', 'committed'].includes(
        t.state,
      ),
    },
    controller:
      t.state === 'review'
        ? null
        : {
            stage: t.state,
            boundary_at: current >= 7 ? now() : null,
            completed: t.state === 'complete',
            source: {
              stopped: current >= 4,
              restart_disabled: current >= 4,
              removed: !!t.original_removed,
            },
          },
  };
}

export function stagesApi(s, c) {
  const { path, method, body, json, fail } = c;
  s.transfers ??= {};
  if (path === '/admin/services' && method === 'GET') {
    const kinds = [
      'seerr',
      'recyclarr',
      'radarr',
      'sonarr',
      'lidarr',
      'bazarr',
      'prowlarr',
      'nzbget',
    ];
    return json({
      items: kinds.map((kind) => {
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
        const stage = owned ? 'owned' : service ? 'connected' : 'none';
        return {
          kind,
          stage,
          hold: transfer && owned ? 'transfer' : null,
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
                operational: !!owned,
                checked_at: service.checked_at ?? now(),
                error: service.error ?? null,
              }
            : null,
          ownership: owned
            ? {
                ...owned.provision,
                hold: transfer ? 'transfer' : null,
                transfer_id: owned.provision.transfer_id ?? null,
              }
            : null,
          operation: transfer
            ? {
                type: 'transfer',
                id: transfer.id,
                state: view(transfer).state,
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
    if (action === 'test')
      return json({ healthy: true, version: service.version });
    if (action === 'forget') {
      s.services = s.services.filter((item) => item !== service);
      return json({ forgotten: true });
    }
    const existing = Object.values(s.transfers).find(
      (item) =>
        item.kind === service.kind &&
        !['complete', 'cancelled', 'returned'].includes(item.state),
    );
    if (existing) return json(view(existing));
    const id2 = `transfer-${service.kind}`;
    s.transfers[id2] = {
      id: id2,
      kind: service.kind,
      service_id: service.id,
      state: 'review',
      step: 'review',
      review: review(service.kind, service),
      review_revision: `revision-${service.kind}`,
      choices: {},
      setup: [],
      recovery: {},
      progress: null,
      error: null,
      replacement_id: null,
    };
    return json(view(s.transfers[id2]));
  }
  if (path === '/admin/transfers' && method === 'GET')
    return json({
      items: Object.values(s.transfers).map((item) => view(item)),
    });
  const transfer = path.match(/^\/admin\/transfers\/([^/]+)(?:\/(.*))?$/);
  if (transfer) {
    const t = s.transfers[transfer[1]];
    if (!t) return fail(404, 'Transfer not found');
    const action = transfer[2];
    if (!action) return json(view(t));
    if (action === 'choices') {
      t.choices = { compose_mode: body.compose_mode };
      t.review_revision = `revision-${t.kind}-${body.compose_mode}`;
      return json(view(t));
    }
    if (action === 'approve') {
      if (body.review_revision !== t.review_revision)
        return fail(
          409,
          'The review changed; check it again before confirming',
        );
      t.state = 'queued';
      t.step = 'stop';
      t.started = Date.now();
      t.complete = () => {
        const service = s.services.find((item) => item.id === t.service_id);
        if (!service) return;
        s.stack = s.stack.filter((item) => item.kind !== t.kind);
        s.stack.push({
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
          can_recreate: false,
          can_retire: false,
          can_remove: false,
          error: null,
          transfer_pending: false,
          origin: 'transferred',
          provision: {
            id: t.id,
            kind: t.kind,
            state: 'complete',
            host_port: null,
            container_id: `replacement-${t.kind}`,
            service_id: t.service_id,
            error: null,
            origin: 'transferred',
            hold: null,
            transfer_id: t.id,
          },
        });
      };
      return json(view(t));
    }
    if (action === 'cancel') {
      t.state = 'cancelled';
      return json(view(t));
    }
    if (action === 'original/remove') {
      t.original_removed = true;
      return json(view(t));
    }
    return fail(409, 'This preview transfer has no failure to recover');
  }
  if (path === '/admin/recyclarr/connection' && method === 'POST')
    return json({ id: 'connected-recyclarr', healthy: true, version: '8.7.2' });
  return false;
}
