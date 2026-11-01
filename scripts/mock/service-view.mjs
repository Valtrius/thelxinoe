// The server's services view (apps/server/src/managers/view.rs), composed from
// the same parts. Each part is { value } or { error }; a missing part is empty.
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
const managerKinds = ['radarr', 'sonarr', 'lidarr'];
const transferStates = {
  queued: ['Transfer queued', 'busy'],
  running: ['Transferring', 'busy'],
  committed: ['Owned · finishing transfer', 'busy'],
  recovery: ['Recovering', 'busy'],
  failed: ['Transfer needs attention', 'bad'],
  suspended: ['Transfer suspended', 'warn'],
};

function label(service, unavailable) {
  const { runtime, integration, provision, operation, connection } = service;
  if (runtime?.registered === false) return ['Setup mismatch', 'warn'];
  if (operation?.type === 'transfer' && transferStates[operation.state])
    return transferStates[operation.state];
  if (service.stage === 'connected')
    return connection?.verified && !connection.error
      ? ['Connected', 'ok']
      : ['Unverified', 'warn'];
  const setup = {
    queued: ['Installing', 'busy'],
    installing: ['Installing', 'busy'],
    connecting: ['Connecting API', 'busy'],
    retiring: ['Retiring', 'busy'],
    blocked: ['Setup blocked', 'bad'],
  }[provision?.state];
  if (setup) return setup;
  if (service.update?.latest?.state === 'activating')
    return ['Connecting API', 'busy'];
  const live = {
    unavailable: ['Status unavailable', 'warn'],
    missing: ['Container missing', 'bad'],
    ready: ['Ready', 'ok'],
  }[runtime?.status];
  if (live) return live;
  if (runtime?.running === false) return ['Stopped', 'muted'];
  if (integration?.error) return ['API unavailable', 'warn'];
  if (runtime?.running === true) return ['Running', 'ok'];
  if (integration) return ['Connected', 'ok'];
  if (unavailable) return ['Status unavailable', 'warn'];
  return ['Not connected', 'muted'];
}

export function serviceView(parts) {
  const errors = Object.fromEntries(
    Object.entries(parts)
      .filter(([, part]) => part?.error !== undefined)
      .map(([name, part]) => [name, part.error]),
  );
  const value = (name) => parts[name]?.value ?? {};
  const { items: managers = [] } = value('managers');
  const { items: support = [] } = value('support');
  const { items: containers = [] } = value('containers');
  const { items: runtimes = [], provisions = [] } = value('stack');
  const { items: stages = [] } = value('stages');
  const { items: connections = [] } = value('connections');
  const updates = value('updates');
  const items = kinds.map((kind) => {
    const manager = managerKinds.includes(kind);
    const target = updates.services?.find((item) => item.kind === kind);
    const provision = provisions.find((item) => item.kind === kind) ?? null;
    const service = {
      ...(stages.find((item) => item.kind === kind) ?? { kind }),
      integration:
        (manager ? managers : support).find((item) => item.kind === kind) ??
        null,
      provision,
      runtime: errors.stack
        ? provision && {
            id: provision.id,
            kind,
            status: 'unavailable',
            existence: 'unknown',
            running: null,
            drift: null,
            can_recreate: false,
            can_retire: false,
            can_remove: false,
          }
        : (runtimes.find((item) => item.kind === kind) ?? null),
      update: target
        ? {
            target,
            policy:
              updates.policies?.find((item) => item.service_id === target.id) ??
              null,
            latest:
              updates.items?.find((item) => item.service_id === target.id) ??
              null,
          }
        : null,
    };
    const [text, tone] = label(
      service,
      !!(errors.stack || errors[manager ? 'managers' : 'support']),
    );
    return { ...service, status: { label: text, tone } };
  });
  return {
    items,
    containers,
    connections,
    timezone: updates.timezone ?? null,
    server_policy: updates.server_policy ?? null,
    errors,
  };
}
