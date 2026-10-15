import type { ServiceRelease } from './ServiceUpdateRelease.svelte';
import type { SupportDownload } from './downloads';
export type ServiceKind =
  | 'radarr'
  | 'sonarr'
  | 'lidarr'
  | 'bazarr'
  | 'prowlarr'
  | 'nzbget'
  | 'seerr'
  | 'recyclarr';
export type Definition = {
  kind: ServiceKind;
  label: string;
  role: 'manager' | 'support' | 'job';
  internalPort: number | null;
  hostPort: number | null;
  description: string;
};
export type ManagerDefaults = {
  root_folder: string;
  quality_profile: number;
  metadata_profile: number | null;
  monitored: boolean;
};
export type ManagerService = {
  id: string;
  name: string;
  kind: ServiceKind;
  container_id: string;
  port: number;
  url_base: string;
  access_url: string | null;
  native_url?: string | null;
  version: string;
  defaults: Partial<ManagerDefaults>;
  retention_enabled?: boolean;
  checked_at: number;
  error: string | null;
};
export type SupportService = {
  id: string;
  name: string;
  kind: ServiceKind;
  container_id: string;
  port: number;
  version: string;
  url_base: string;
  access_url: string | null;
  native_url?: string | null;
  checked_at: number;
  error?: string | null;
};
export type ManagerOptions = {
  defaults: ManagerDefaults;
  roots: { id: number; path: string }[];
  profiles: {
    id: number;
    name: string;
    trash_id?: string | null;
    url?: string | null;
  }[];
  metadata_profiles: { id: number; name: string }[];
};
export type SupportSnapshot = {
  health?: ({ message: string; type: string } | string)[];
  indexers?: {
    id: number;
    name: string;
    enabled: boolean;
    disabled_until: string | null;
  }[];
  queue?: SupportDownload[];
  history?: SupportDownload[];
  rate?: number;
  limit?: number;
  paused?: boolean;
  free_mb?: number;
};
export type StackService = {
  id: string;
  registered?: boolean;
  kind: ServiceKind;
  name: string;
  phase: string;
  image: string;
  drift: boolean | null;
  running: boolean | null;
  existence: 'present' | 'missing' | 'unknown';
  status:
    'running' | 'stopped' | 'missing' | 'unavailable' | 'drifted' | 'ready';
  inspection_error: string | null;
  can_recreate: boolean;
  can_retire: boolean;
  can_remove: boolean;
  error: string | null;
  transfer_pending: boolean;
  hold?: string | null;
  origin?: 'installed' | 'transferred';
};
export type Provision = {
  id: string;
  kind: ServiceKind;
  state: string;
  host_port: number | null;
  container_id: string | null;
  service_id: string | null;
  error: string | null;
  origin: 'installed' | 'transferred';
  hold: 'setup' | 'transfer' | 'recovery' | null;
  transfer_id: string | null;
  operation?: 'remove' | 'retire' | null;
};
export type UpdatePolicy = {
  service_id: string;
  policy: string;
  window_start: number;
  window_end: number;
  candidate: string | null;
  release?: ServiceRelease | null;
  checked_at: number;
  error: string | null;
};
export type Update = {
  id: string;
  service_id: string;
  state: string;
  candidate: string | null;
  error: string | null;
  classification?: string;
};
export type UpdateTarget = { id: string; kind: ServiceKind };
export type ServerUpdatePolicy = {
  policy: string;
  window_start: number;
  window_end: number;
};
export type ApprovalUser = { id: string; username: string; enabled: boolean };
export type SetupDraft = {
  name: string;
  container: string;
  port: number | null;
  username: string;
  secret: string;
  urlBase: string;
};
export type InstallDraft = { hostPort: number | null };
export type DefaultsDraft = ManagerDefaults & { loaded: boolean };
export type UpdateDraft = {
  targetId: string | null;
  policy: string;
  start: number;
  end: number;
};

export const definitions: Definition[] = [
  {
    kind: 'seerr',
    label: 'Seerr',
    role: 'support',
    internalPort: 5055,
    hostPort: 15055,
    description: 'Discovery and requests',
  },
  {
    kind: 'recyclarr',
    label: 'Recyclarr',
    role: 'job',
    internalPort: null,
    hostPort: null,
    description: 'TRaSH Guides sync',
  },
  {
    kind: 'radarr',
    label: 'Radarr',
    role: 'manager',
    internalPort: 7878,
    hostPort: 17878,
    description: 'Movie manager',
  },
  {
    kind: 'sonarr',
    label: 'Sonarr',
    role: 'manager',
    internalPort: 8989,
    hostPort: 18989,
    description: 'Series manager',
  },
  {
    kind: 'lidarr',
    label: 'Lidarr',
    role: 'manager',
    internalPort: 8686,
    hostPort: 18686,
    description: 'Music manager',
  },
  {
    kind: 'bazarr',
    label: 'Bazarr',
    role: 'support',
    internalPort: 6767,
    hostPort: 16767,
    description: 'Subtitles',
  },
  {
    kind: 'prowlarr',
    label: 'Prowlarr',
    role: 'support',
    internalPort: 9696,
    hostPort: 19696,
    description: 'Indexers',
  },
  {
    kind: 'nzbget',
    label: 'NZBGet',
    role: 'support',
    internalPort: 6789,
    hostPort: 16789,
    description: 'Usenet downloads',
  },
];
export const stages: Record<string, string> = {
  queued: 'Waiting to check',
  submitting: 'Starting check',
  preparing: 'Preparing',
  snapshotting: 'Saving appdata',
  preflight: 'Checking compatibility',
  ready: 'Compatible, ready to install',
  'queued-activate': 'Waiting to install',
  'recovery-snapshot': 'Saving recovery copy',
  'isolated-live-validation': 'Validating before activation',
  'rollback-copying': 'Restoring previous service',
  'rollback-activating': 'Restarting previous service',
  activating: 'Starting updated service',
  committed: 'Update complete',
  'rolled-back': 'Previous service restored',
  blocked: 'Needs attention',
  'recovery-required': 'Recovery required',
  'runtime-failure': 'Updated service needs attention',
  'queued-recover': 'Waiting to restore',
};
export const loaderSubsystems = [
  { key: 'managers', label: 'Acquisition managers' },
  { key: 'support', label: 'Support services' },
  { key: 'approval', label: 'Request approval settings' },
  { key: 'updates', label: 'Service updates' },
  { key: 'containers', label: 'Docker container discovery' },
  { key: 'stack', label: 'Managed service runtime' },
  { key: 'connections', label: 'Optional service connections' },
  { key: 'stages', label: 'Service stages' },
] as const;

/** Thelxinoe's two service stages; setup, transfer and recovery are operations. */
export type ServiceStageName = 'none' | 'connected' | 'owned';
export type ServiceStage = {
  kind: ServiceKind;
  stage: ServiceStageName;
  hold: 'setup' | 'transfer' | 'recovery' | null;
  agreed: boolean;
  draft_review: string | null;
  actions: string[];
  connection: {
    id: string;
    container_id: string;
    container_name: string;
    engine: string;
    version: string;
    verified: boolean;
    operational: boolean;
    checked_at: number;
    error: string | null;
  } | null;
  ownership: {
    id: string;
    state: string;
    origin: 'installed' | 'transferred';
    hold: string | null;
    transfer_id: string | null;
  } | null;
  operation: {
    type: 'transfer' | 'install';
    id: string;
    state: string;
    step?: string;
    error: string | null;
  } | null;
};
export type TransferStepStatus =
  | 'complete'
  | 'running'
  | 'awaiting_input'
  | 'blocked'
  | 'failed'
  | 'skipped'
  | 'pending';
export type TransferStep = {
  key: string;
  label: string;
  actor: 'user' | 'backend';
  status: TransferStepStatus;
};
export type TransferCheck = {
  id: string;
  label: string;
  required: boolean;
  status: 'pending' | 'passed' | 'failed' | 'skipped';
  detail: string | null;
  evidence: unknown;
};
export type TransferChange = {
  area?: string;
  setting?: string;
  before: unknown;
  after: unknown;
  reason: string;
};
export type TransferMount = {
  kind: string;
  source: string;
  destination: string;
  writable: boolean;
  role: string;
};
export type TransferPlan = {
  review_id: string;
  revision: string;
  kind: ServiceKind;
  source: {
    container_id: string;
    name: string;
    image: string;
    version: string;
    running: boolean;
    restart_policy: string;
    networks: string[] | null;
    appdata: TransferMount;
    identity: { uid: number; gid: number };
  };
  target: {
    image: string;
    version: string;
    decision: { decision: 'same' | 'upgrade'; image: string; reason?: string };
    appdata: string;
    appdata_destination: string;
    identity: { uid: number; gid: number };
    aliases: string[];
    published_port: number | null;
    mounts: TransferMount[];
    supplementary: TransferMount[];
  };
  changes: TransferChange[];
  settings: TransferChange[];
  automation: {
    kind: 'updater' | 'docker_access';
    name: string;
    scope: string;
    blocks: boolean;
  }[];
  compose: {
    project: string;
    service: string | null;
    working_dir: string | null;
    config_files: string | null;
    unrelated: { name: string; service: string | null; running: boolean }[];
  } | null;
  blockers: string[];
};
export type TransferView = {
  id: string;
  kind: ServiceKind;
  service_id: string;
  state:
    | 'review'
    | 'queued'
    | 'running'
    | 'committed'
    | 'failed'
    | 'recovery'
    | 'suspended'
    | 'complete'
    | 'cancelled'
    | 'returned';
  step: string;
  steps: TransferStep[];
  review: {
    plan: TransferPlan;
    blockers: string[];
    links: {
      kind: string;
      peer: string;
      peer_kind: ServiceKind;
      direction: string;
      action: 'connect_after_completion' | 'stays_disconnected';
    }[];
    dependencies: {
      host: string;
      kind: ServiceKind | null;
      status: string;
      detail: string;
    }[];
    checks: TransferCheck[];
    setup_actions: { action: string; label: string }[];
    recyclarr_targets: {
      kind: string;
      name: string;
      rewrite: TransferChange;
    }[];
  };
  review_revision: string;
  choices: { compose_mode?: 'scoped' | 'project' | null };
  checks: TransferCheck[];
  setup: { action: string; label: string; state: string; error?: string }[];
  recovery: (TransferRecovery | Record<string, never>) & {
    command?: TransferRecoveryCommand;
  };
  progress: {
    phase?: string;
    bytes?: number;
    files?: number;
    entries?: number;
    waiting?: string;
  } | null;
  error: string | null;
  replacement_id: string | null;
  actions: {
    approve: boolean;
    cancel: boolean;
    retry: boolean;
    restore: boolean;
    recovery: boolean;
    remove_original: boolean;
    continue_in_background: boolean;
  };
  controller: {
    stage: string;
    boundary_at: number | null;
    completed: boolean;
    source: { stopped: boolean; restart_disabled: boolean; removed: boolean };
    command?: { operation: string; phase: string; state: string } | null;
  } | null;
};
/** An approved recovery the controller runs and journals phase by phase. */
export type TransferRecoveryCommand = {
  operation: string;
  action: string;
  outcome?: 'applied' | 'failed' | 'refused';
};
export type TransferRecovery = {
  revision: string;
  evidence: {
    boundary_at: number;
    images: { source: string; replacement: string };
    versions: { source: string; replacement: string };
    containers: {
      replacement: { present: boolean; running?: boolean };
      original: { present: boolean; running?: boolean; unchanged?: boolean };
    };
    effects: {
      known: { effect: string; count: number }[];
      uncertain: string[];
    };
  };
  options: {
    action: string;
    label: string;
    available: boolean;
    /** Why an unavailable option can't run now. */
    unavailable?: string | null;
    description: string;
    requires?: string[];
  }[];
  cannot_undo: string[];
  transfer_written: { action: string; label: string; state: string }[];
  peer_note: string;
};
export type TransferDeclarations = {
  cron: boolean;
  systemd: boolean;
  ci_cd: boolean;
  configuration_management: boolean;
  manual_compose: boolean;
  unobservable_automation: boolean;
  scoped_removed_from_files: boolean;
  scoped_no_orphan_removal: boolean;
  scoped_no_restart: boolean;
  scoped_no_appdata_writes: boolean;
  project_retired: boolean;
  unrelated_services_acknowledged: boolean;
};
