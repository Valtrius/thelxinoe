import type { Job, JobPage } from './api';
import { definitions } from './services/feature';

export const jobKinds: Record<string, string> = {
  'library.scan': 'Scan library',
  'metadata.refresh': 'Refresh metadata',
  'metadata.match': 'Match metadata',
  'manager.request': 'Acquire requested media',
  'recyclarr.sync': 'Sync Recyclarr',
  'stack.install': 'Install service',
  'service.update': 'Update service',
  'service.transfer': 'Transfer service',
};

const actionLabels: Record<string, string> = {
  'service.update:preflight': 'Check service update',
  'service.update:recover': 'Recover service update',
  'recyclarr.sync:preview': 'Preview Recyclarr sync',
};

/** A readable label; unknown kinds are spelled out from their segments. */
export function jobLabel(job: Pick<Job, 'kind' | 'action'>): string {
  const label = actionLabels[`${job.kind}:${job.action}`] ?? jobKinds[job.kind];
  if (label) return label;
  const words = job.kind.replace(/[._-]+/g, ' ').trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** What the job acted on, or null once that no longer exists. */
export function jobTarget(job: Pick<Job, 'target' | 'service'>): string | null {
  if (job.service)
    return (
      definitions.find((service) => service.kind === job.service)?.label ??
      job.service
    );
  return job.target;
}

/** Who started the job, or null when that was never recorded. */
export function jobTrigger(
  job: Pick<Job, 'automatic' | 'actor'>,
): string | null {
  if (job.automatic === null) return null;
  return job.automatic ? 'Automatic' : (job.actor ?? 'Deleted user');
}

/** Timestamps are whole seconds, so anything shorter reads as under one. */
export function formatJobDuration(seconds: number): string {
  const total = Math.floor(seconds);
  if (total < 1) return '<1 s';
  if (total < 60) return `${total} s`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60)
    return total % 60 ? `${minutes} m ${total % 60} s` : `${minutes} m`;
  const hours = Math.floor(minutes / 60);
  return minutes % 60 ? `${hours} h ${minutes % 60} m` : `${hours} h`;
}

/** Seconds spent running, counting up while it runs, or null if unknown. */
export function jobRunTime(
  job: Pick<Job, 'state' | 'started_at' | 'completed_at'>,
  now: number,
): number | null {
  if (job.started_at === null) return null;
  if (job.state === 'running') return Math.max(0, now - job.started_at);
  if (job.completed_at === null) return null;
  return Math.max(0, job.completed_at - job.started_at);
}

export type JobFilters = {
  kind: string;
  state: Job['state'] | '';
  showAutomatic: boolean;
};

export const jobPageSize = 100;

export function jobsQuery(
  filters: JobFilters,
  range: { limit?: number; since?: number } = {},
): string {
  const query = new URLSearchParams();
  if (filters.kind) query.set('kind', filters.kind);
  if (filters.state) query.set('state', filters.state);
  if (!filters.showAutomatic) query.set('hide_automatic', 'true');
  if (range.limit) query.set('limit', String(range.limit));
  if (range.since) query.set('since', String(range.since));
  return query.toString();
}

/** Whether more jobs match than the newest ones listed. */
export function hasOlderJobs(
  page: JobPage,
  state: JobFilters['state'],
): boolean {
  const matching = state
    ? page.totals[state]
    : Object.values(page.totals).reduce((sum, count) => sum + count, 0);
  return page.items.length < matching;
}
