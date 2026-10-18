import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ServerUpdateStatus } from './server-updates';

const { call, source } = vi.hoisted(() => ({
  call: vi.fn(),
  source: { url: 'https://server.test' },
}));
vi.mock('./api', () => ({
  api: call,
  desktop: true,
  serverUrl: () => source.url,
  ApiError: class extends Error {
    constructor(
      public status: number,
      public code: string,
      message: string,
    ) {
      super(message);
    }
  },
}));
let updates: typeof import('./server-updates');
let status: ServerUpdateStatus;
let stop: () => void;
let observed: { error: string; status: ServerUpdateStatus | null };

beforeEach(async () => {
  vi.resetModules();
  vi.stubGlobal('window', new EventTarget());
  source.url = 'https://server.test';
  status = {
    version: '0.1.0',
    timezone: 'UTC',
    configured: true,
    policy: { policy: 'notify', window_start: 3, window_end: 5 },
    release: { version: '0.1.1', notes: '' },
    observation: { checked_at: 1, error: null },
    controller: { items: [] },
  };
  call.mockReset().mockImplementation(async (_path: string, method = 'GET') => {
    if (method === 'POST') throw Error('Connection lost');
    return structuredClone(status);
  });
  updates = await import('./server-updates');
  stop = updates.serverUpdates.subscribe((value) => {
    observed = value;
  });
  await vi.waitFor(() => expect(observed.status?.version).toBe('0.1.0'));
});
afterEach(() => {
  stop?.();
  vi.unstubAllGlobals();
});

function intent(id: string, state = 'pending') {
  return {
    id,
    state,
    version: '0.1.1',
    previous_version: '0.1.0',
    error: null,
  };
}

describe('desktop server update response reconciliation', () => {
  it('does not mistake a historical request for acknowledgement of a new attempt', async () => {
    status.request = intent('old', 'failed');
    await updates.refreshServerUpdate();
    await updates.serverUpdateAction('install');
    expect(observed.error).toBe('Connection lost');
    await updates.refreshServerUpdate();
    expect(observed.error).toBe('Connection lost');
    status.request = intent('new', 'failed');
    status.request.error = 'Validation failed';
    await updates.refreshServerUpdate();
    expect(observed.error).toBe('');
    expect(observed.status?.request?.error).toBe('Validation failed');
  });
  it('preserves a definitive rejection and ignores another release', async () => {
    const { ApiError } = await import('./api');
    call.mockImplementation(async (_path: string, method = 'GET') => {
      if (method === 'POST')
        throw new ApiError(409, 'conflict', 'Release changed');
      return structuredClone(status);
    });
    await updates.serverUpdateAction('install');
    status.request = intent('unrelated');
    await updates.refreshServerUpdate();
    expect(observed.error).toBe('Release changed');
  });
  it('does not reconcile an acknowledgement from a different server', async () => {
    await updates.serverUpdateAction('install');
    source.url = 'https://other.test';
    status.request = intent('other-server-request');
    await updates.refreshServerUpdate();
    expect(observed.error).toBe('Connection lost');
  });
});

describe('web tab reload readiness', () => {
  const item = (stage: string, version = '0.1.1', previous = '0.1.0') => ({
    id: `${version}-${stage}`,
    version,
    previous_version: previous,
    stage,
    error: null,
    snapshot_ready: true,
    recovery_tested: true,
    activation_crossed: true,
  });
  it('waits for the controller to commit or restore the served version', () => {
    status.version = '0.1.1';
    status.controller.items = [item('activating')];
    expect(updates.webUpdateSettled(status, '0.1.1')).toBe(false);
    status.controller.items = [item('committed')];
    expect(updates.webUpdateSettled(status, '0.1.1')).toBe(true);
    expect(updates.webUpdateSettled(status, '0.1.2')).toBe(false);
    status.version = '0.1.0';
    status.controller.items = [item('recovering')];
    expect(updates.webUpdateSettled(status, '0.1.0')).toBe(false);
    status.controller.items = [item('restored')];
    expect(updates.webUpdateSettled(status, '0.1.0')).toBe(true);
  });
  it('trusts the server version without controller state or history', () => {
    status.version = '0.1.2';
    status.controller = { items: [item('activating')], error: 'Unavailable' };
    expect(updates.webUpdateSettled(status, '0.1.2')).toBe(true);
    // Installed outside the controller, e.g. by recreating the containers.
    status.controller = { items: [item('committed')] };
    expect(updates.webUpdateSettled(status, '0.1.2')).toBe(true);
  });
});
