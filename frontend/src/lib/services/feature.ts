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
  version: string;
  defaults: Partial<ManagerDefaults>;
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
};
export type Provision = {
  id: string;
  kind: ServiceKind;
  state: string;
  host_port: number | null;
  container_id: string | null;
  service_id: string | null;
  error: string | null;
  origin: string;
};
export type TransferReview = {
  review_id: string;
  name: string;
  image: string;
  source_config: string;
  managed_config: string;
  compose_project: string | null;
  compose_service: string | null;
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
] as const;
