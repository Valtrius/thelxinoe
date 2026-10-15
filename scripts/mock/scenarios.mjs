import { definitions, label } from './data.mjs';
import { settingsSeed } from './settings.mjs';
import { serverActionDelay } from './timing.mjs';
import {
  advanceTransfer,
  createTransfer,
  setTransferStep,
  syncTransfers,
  transferView,
} from './transfers.mjs';

const presets = [
  ['owned', 'Owned'],
  ['unconfigured', 'Unconfigured'],
  ['connected', 'Connected'],
  ['review', 'Ownership review'],
  ['blocked', 'Blocked prerequisites'],
  ['copy', 'Copying appdata'],
  ['validate', 'Validating copy'],
  ['held', 'Owned, operations held'],
  ['complete', 'Transfer complete'],
  ['failed-before', 'Failure before activation'],
  ['failed-after', 'Failure after activation'],
  ['recovery-unavailable', 'Recovery unavailable'],
];
const kinds = definitions
  .filter(([, , role]) => role !== 'job')
  .map(([kind]) => kind);
const defaults = () => ({
  preset: 'owned',
  result: 'success',
  delay: serverActionDelay,
  linkState: 'connected',
  outcome: 'success',
  paused: false,
});

export function scenarioSeed() {
  return {
    services: Object.fromEntries(kinds.map((kind) => [kind, defaults()])),
  };
}

export function scenarioView(s) {
  syncTransfers(s);
  return {
    kinds: kinds.map((kind) => ({ kind, label: label(kind) })),
    presets: presets.map(([value, text]) => ({ value, label: text })),
    services: s.scenarios.services,
    transfers: Object.fromEntries(
      Object.values(s.transfers ?? {}).map((t) => [t.kind, transferView(t)]),
    ),
  };
}

export function setLinkState(s, kind, state) {
  for (const link of s.connections.filter(
    (item) => item.source_kind === kind,
  )) {
    link.state = state === 'cleanup' ? 'unavailable' : state;
    link.enabled = !['available', 'cleanup'].includes(state);
    link.cleanup_pending = state === 'cleanup';
    link.error =
      state === 'conflict'
        ? 'An existing application record already reaches this service.'
        : ['unavailable', 'cleanup'].includes(state)
          ? 'The source service is temporarily unreachable.'
          : null;
    link.candidate =
      state === 'conflict'
        ? {
            name: `Existing ${link.target_name}`,
            address: `http://${link.target_kind}:7878`,
          }
        : null;
  }
}

function loadScenario(s, kind, preset) {
  const seed = settingsSeed();
  s.services = s.services.filter((item) => item.kind !== kind);
  s.stack = s.stack.filter((item) => item.kind !== kind);
  s.transfers ??= {};
  for (const [id, t] of Object.entries(s.transfers))
    if (t.kind === kind) delete s.transfers[id];
  s.scenarios.services[kind] = defaults();
  const config = s.scenarios.services[kind];
  config.preset = preset;
  const service = seed.services.find((item) => item.kind === kind);
  if (preset !== 'unconfigured') s.services.push(service);
  if (preset === 'owned')
    s.stack.push(seed.stack.find((item) => item.kind === kind));
  if (service.defaults) s.profiles[service.id] = seed.profiles[service.id];
  setLinkState(s, kind, 'connected');
  if (['owned', 'unconfigured', 'connected'].includes(preset)) return;
  const t = createTransfer(s, service);
  if (preset === 'blocked') {
    t.step = 'prerequisites';
    t.review.blockers = [
      'The original service has active downloads. Wait for it to become idle.',
    ];
    t.review.plan.blockers = [...t.review.blockers];
  } else if (preset !== 'review') {
    config.outcome =
      preset === 'failed-before'
        ? 'before'
        : preset === 'failed-after'
          ? 'after'
          : preset === 'recovery-unavailable'
            ? 'unavailable'
            : 'success';
    setTransferStep(
      s,
      t,
      {
        held: 'commit',
        'failed-before': 'validate',
        'failed-after': 'checks',
        'recovery-unavailable': 'checks',
      }[preset] ?? preset,
    );
  }
}

export function controlScenario(s, body) {
  if (body.action === 'reset') {
    const seed = settingsSeed();
    for (const key of [
      'services',
      'stack',
      'connections',
      'profiles',
      'updates',
    ])
      s[key] = seed[key];
    s.transfers = {};
    s.scenarios = scenarioSeed();
    return;
  }
  const config = s.scenarios.services[body.kind];
  if (!config) throw Error('Unknown preview service');
  if (body.action === 'scenario') {
    if (!presets.some(([value]) => value === body.preset))
      throw Error('Unknown service scenario');
    loadScenario(s, body.kind, body.preset);
  } else if (body.action === 'settings') {
    if (
      !['success', 'credentials', 'unreachable'].includes(body.result) ||
      ![0, 500, 1500, 3000, 5000].includes(body.delay) ||
      ![
        'connected',
        'available',
        'pending',
        'unavailable',
        'conflict',
        'cleanup',
      ].includes(body.linkState) ||
      !['success', 'before', 'after', 'unavailable'].includes(body.outcome) ||
      typeof body.paused !== 'boolean'
    )
      throw Error('Invalid service controls');
    syncTransfers(s);
    Object.assign(config, {
      result: body.result,
      delay: body.delay,
      linkState: body.linkState,
      outcome: body.outcome,
      paused: body.paused,
    });
    for (const t of Object.values(s.transfers ?? {}))
      if (t.kind === body.kind) t.updated = Date.now();
    setLinkState(s, body.kind, config.linkState);
  } else if (body.action === 'advance') {
    for (const t of Object.values(s.transfers ?? {}))
      if (t.kind === body.kind) advanceTransfer(s, t);
  } else throw Error('Unknown preview action');
}

export function serviceAttention(s) {
  syncTransfers(s);
  const items = [];
  const activity = [];
  for (const t of Object.values(s.transfers ?? {})) {
    const message = `${label(t.kind)}: ${t.error ?? t.step}`;
    if (['failed', 'suspended'].includes(t.state))
      items.push({
        id: t.id,
        severity: 'warning',
        target: 'services',
        resource: t.kind,
        message,
        revision: `${t.state}-${t.step}`,
        dismissible: false,
        media_ids: [],
      });
    if (['queued', 'running', 'committed', 'recovery'].includes(t.state))
      activity.push({
        id: t.id,
        target: 'services',
        resource: t.kind,
        state: 'running',
        message,
        media_ids: [],
      });
  }
  for (const link of s.connections) {
    if (link.state === 'pending' && link.enabled)
      activity.push({
        id: `connection:${link.id}`,
        target: 'services',
        resource: link.source_kind,
        state: 'running',
        message: `Connecting ${label(link.source_kind)} to ${link.target_name}`,
        media_ids: [],
      });
    if (link.error && (link.enabled || link.cleanup_pending))
      items.push({
        id: `connection:${link.id}`,
        target: 'services',
        resource: link.source_kind,
        severity: 'warning',
        message: link.error,
        revision: link.state,
        dismissible: false,
        media_ids: [],
      });
  }
  return { items, activity };
}
