import { describe, expect, it } from 'vitest';
import type { Job, JobPage } from './api';
import {
  formatJobDuration,
  hasOlderJobs,
  jobLabel,
  jobRunTime,
  jobsQuery,
  jobTarget,
  jobTrigger,
} from './jobs';

const job = (id: string, created_at: number): Job => ({
  id,
  kind: 'library.scan',
  action: null,
  state: 'complete',
  attempts: 1,
  error: null,
  created_at,
  available_at: created_at,
  started_at: created_at,
  completed_at: created_at,
  target: null,
  service: null,
  automatic: null,
  actor: null,
});
const page = (items: Job[], totals: JobPage['totals']): JobPage => ({
  items,
  pinned: [],
  totals,
});

describe('job presentation', () => {
  it('labels kinds, their actions and unknown kinds', () => {
    expect(jobLabel({ kind: 'library.scan', action: null })).toBe(
      'Scan library',
    );
    expect(jobLabel({ kind: 'service.update', action: 'activate' })).toBe(
      'Update service',
    );
    expect(jobLabel({ kind: 'service.update', action: 'preflight' })).toBe(
      'Check service update',
    );
    expect(jobLabel({ kind: 'recyclarr.sync', action: 'preview' })).toBe(
      'Preview Recyclarr sync',
    );
    expect(jobLabel({ kind: 'future.job-kind', action: null })).toBe(
      'Future job kind',
    );
  });

  it('names services by their label and leaves removed targets empty', () => {
    expect(jobTarget({ service: 'nzbget', target: null })).toBe('NZBGet');
    expect(jobTarget({ service: 'future', target: null })).toBe('future');
    expect(jobTarget({ service: null, target: 'Movies' })).toBe('Movies');
    expect(jobTarget({ service: null, target: null })).toBeNull();
  });

  it('tells automatic, user, deleted-user and unrecorded triggers apart', () => {
    expect(jobTrigger({ automatic: true, actor: null })).toBe('Automatic');
    expect(jobTrigger({ automatic: false, actor: 'Alex' })).toBe('Alex');
    expect(jobTrigger({ automatic: false, actor: null })).toBe('Deleted user');
    expect(jobTrigger({ automatic: null, actor: null })).toBeNull();
  });

  it('formats durations from under a second to hours', () => {
    expect(
      [0, 0.4, 1, 59, 60, 69, 3600, 3660].map((seconds) =>
        formatJobDuration(seconds),
      ),
    ).toEqual([
      '<1 s',
      '<1 s',
      '1 s',
      '59 s',
      '1 m',
      '1 m 9 s',
      '1 h',
      '1 h 1 m',
    ]);
  });

  it('counts running time up and leaves unknown times empty', () => {
    const times = { started_at: 100, completed_at: 130 };
    expect(jobRunTime({ ...times, state: 'complete' }, 500)).toBe(30);
    expect(
      jobRunTime(
        { state: 'running', started_at: 100, completed_at: null },
        142,
      ),
    ).toBe(42);
    // A clock behind the server's never shows negative time.
    expect(
      jobRunTime({ state: 'running', started_at: 100, completed_at: null }, 90),
    ).toBe(0);
    expect(
      jobRunTime({ state: 'queued', started_at: null, completed_at: null }, 90),
    ).toBeNull();
    // Retired jobs finish without a completion time.
    expect(
      jobRunTime(
        { state: 'complete', started_at: 100, completed_at: null },
        90,
      ),
    ).toBeNull();
  });
});

describe('job queries', () => {
  it('sends only the filters in use', () => {
    expect(jobsQuery({ kind: '', state: '', showAutomatic: false })).toBe(
      'hide_automatic=true',
    );
    expect(jobsQuery({ kind: '', state: '', showAutomatic: true })).toBe('');
    expect(
      jobsQuery(
        { kind: 'library.scan', state: 'failed', showAutomatic: true },
        { limit: 200, since: 1789884000 },
      ),
    ).toBe('kind=library.scan&state=failed&limit=200&since=1789884000');
  });
});

describe('older jobs', () => {
  const listed = [job('a', 2), job('b', 1)];
  const totals = { running: 1, queued: 0, failed: 2, complete: 1 };

  it('compares the listed jobs with everything matching', () => {
    expect(hasOlderJobs(page(listed, totals), '')).toBe(true);
    expect(
      hasOlderJobs(page([...listed, job('c', 0), job('d', 0)], totals), ''),
    ).toBe(false);
  });

  it('compares only with the chosen state when one is set', () => {
    expect(hasOlderJobs(page(listed, totals), 'failed')).toBe(false);
    expect(hasOlderJobs(page(listed.slice(0, 1), totals), 'failed')).toBe(true);
  });
});
