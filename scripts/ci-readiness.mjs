import { setTimeout as delay } from 'node:timers/promises';

export const budgets = {
  request: 30000,
  startup: 180000,
  restart: 90000,
  provision: 300000,
  stateCopy: 300000,
};
export const requestBudget = (path) =>
  path.startsWith('/admin/recyclarr/adopt') ||
  /^\/admin\/backups\/[^/]+\/restore$/.test(path)
    ? budgets.stateCopy
    : budgets.request;

export async function waitForState(
  label,
  read,
  accept,
  { timeout = budgets.startup, interval = 1000 } = {},
) {
  const deadline = Date.now() + timeout;
  let observed, lastError;
  while (Date.now() < deadline) {
    try {
      observed = await read(Math.max(1, Math.min(5000, deadline - Date.now())));
      lastError = null;
      if (accept(observed)) return observed;
    } catch (error) {
      if (error.fatal) throw error;
      lastError = error;
    }
    await delay(Math.min(interval, Math.max(0, deadline - Date.now())));
  }
  throw Error(
    `${label} timed out after ${timeout} ms; last state: ${JSON.stringify(observed) ?? 'unavailable'}${lastError ? `; ${lastError.message}` : ''}`,
    { cause: lastError },
  );
}

export async function waitForProvision(read, id, label) {
  let stack;
  const provision = await waitForState(
    label,
    async (timeout) => {
      stack = await read(timeout);
      const item = stack.provisions.find((value) => value.id === id);
      if (item?.state === 'blocked' || item?.state === 'failed')
        throw Object.assign(Error(`${label}: ${item.error ?? item.state}`), {
          fatal: true,
        });
      return item
        ? { id: item.id, state: item.state, error: item.error }
        : null;
    },
    (value) => value?.state === 'complete',
    { timeout: budgets.provision, interval: 1500 },
  );
  return { stack, provision };
}

export function serverToolsReady({ supported, items }) {
  return (
    supported &&
    items.length > 0 &&
    items.every(
      (tool) =>
        tool.installed &&
        !tool.integrity_error &&
        (!['bootstrap', 'validate'].includes(tool.job?.action) ||
          ['complete', 'failed', 'canceled'].includes(tool.job.stage)),
    )
  );
}

export function waitForServerTools(read) {
  return waitForState('Server tools ready', read, serverToolsReady);
}

export function waitForRestart(read, before, urlBase) {
  return waitForState(
    'Service restart',
    read,
    (status) =>
      !!status?.startTime &&
      status.startTime !== before &&
      status.urlBase === urlBase,
    { timeout: budgets.restart, interval: 2000 },
  );
}

export async function waitForJob(
  read,
  expected,
  label,
  {
    timeout = budgets.provision,
    failures = ['blocked', 'failed', 'partial'],
  } = {},
) {
  let job;
  await waitForState(
    label,
    async (requestTimeout) => {
      job = await read(requestTimeout);
      if (job?.state !== expected && failures.includes(job?.state))
        throw Object.assign(
          Error(`${label}: ${job.state}: ${job.error ?? ''}`),
          { fatal: true },
        );
      return job ? { id: job.id, state: job.state, error: job.error } : null;
    },
    (value) => value?.state === expected,
    { timeout, interval: 1500 },
  );
  return job;
}

export function waitForViewportFit(page) {
  return waitForState(
    'Viewport fit',
    () =>
      page.evaluate(() => ({
        content: document.documentElement.scrollWidth,
        viewport: window.innerWidth,
      })),
    (value) => value.content <= value.viewport,
    { timeout: 10000, interval: 100 },
  );
}
