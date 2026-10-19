import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  ServerUpdate,
  ServerUpdateModel,
  ServerUpdateStatus,
} from './server-updates';

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
  it('ignores preparations, which never change the running version', () => {
    status.version = '0.0.2';
    status.controller = nas().controller;
    expect(updates.webUpdateSettled(status, '0.0.2')).toBe(true);
    // A blocked attempt at the next release, prepared from this one.
    status.controller = {
      generation: 2,
      items: [{ ...blocked('next', 2), version: '0.0.3' }],
    };
    expect(updates.webUpdateSettled(status, '0.0.2')).toBe(true);
  });
});

function blocked(id: string, generation = 1): ServerUpdate {
  return {
    id,
    stage: 'blocked',
    error: 'Docker inspection unavailable',
    version: '0.0.2',
    previous_version: '0.0.1',
    source_generation: generation,
    snapshot_ready: false,
    recovery_tested: false,
    activation_crossed: false,
    download: { component: 'controller', received: 0, total: null },
  };
}
// The NAS after two in-app attempts failed at preflight and the administrator
// updated with `docker compose pull && up -d`: the controller adopted generation 2.
function nas(): ServerUpdateStatus {
  return {
    version: '0.0.2',
    timezone: 'UTC',
    configured: true,
    policy: { policy: 'automatic', window_start: 3, window_end: 5 },
    release: null,
    request: {
      id: 'first',
      version: '0.0.2',
      previous_version: '0.0.1',
      state: 'failed',
      error: 'Docker inspection unavailable',
    },
    observation: { checked_at: 1, error: null },
    controller: {
      generation: 2,
      items: [blocked('second'), blocked('first')],
    },
  };
}

describe('product version card', () => {
  const card = (
    status: ServerUpdateStatus | null,
    model: Partial<ServerUpdateModel> = {},
  ) =>
    updates.serverUpdateCard({
      status,
      offlineSince: 0,
      controllerSince: 0,
      updating: false,
      now: Date.now(),
      action: null,
      error: '',
      ...model,
    });
  it('shows the running version once attempts for old containers are obsolete', () => {
    const shown = card(nas());
    expect(shown.version).toBe('0.0.2');
    expect(shown.view.title).toBe('Up to date');
    expect(shown.dismissible).toBeNull();
    // The controller retires them itself from now on.
    const retired = nas();
    retired.controller = {
      items: retired.controller.items.map((item) => ({
        ...item,
        stage: 'superseded',
      })),
    };
    expect(card(retired).view.title).toBe('Up to date');
  });
  it('offers to dismiss a failed attempt and then hides it', () => {
    const status = nas();
    status.version = '0.0.1';
    status.controller.generation = 1;
    const failed = card(status);
    expect(failed.version).toBe('0.0.1');
    expect(failed.view.title).toBe('Update could not complete');
    expect(failed.dismissible).toBe('second');
    status.dismissed = 'second';
    expect(card(status).view.title).toBe('Up to date');
    // A restart only interrupted it; it starts again by itself.
    status.dismissed = null;
    status.controller.items[0].failure = 'interrupted';
    expect(card(status).dismissible).toBeNull();
  });
  it('keeps the previous version until the controller accepts the new one', () => {
    status.version = '0.1.1';
    status.request = intent('update');
    status.controller = {
      generation: 1,
      items: [
        {
          ...blocked('update'),
          version: '0.1.1',
          previous_version: '0.1.0',
          stage: 'activating',
        },
      ],
    };
    expect(card(status).version).toBe('0.1.0');
    expect(card(status).cancellable).toBe(false);
    // The new server answers while its controller restarts.
    status.controller = { items: [], error: 'Controller unavailable' };
    const reconnecting = card(status, {
      updating: true,
      controllerSince: Date.now(),
    });
    expect(reconnecting.version).toBe('0.1.0');
    expect(reconnecting.view.title).toBe(
      'Reconnecting to the update controller',
    );
    status.controller = {
      items: [
        {
          ...blocked('update'),
          version: '0.1.1',
          previous_version: '0.1.0',
          stage: 'committed',
        },
      ],
    };
    status.request = { ...intent('update'), state: 'completed' };
    status.release = null;
    expect(card(status).version).toBe('0.1.1');
    expect(card(status).view.title).toBe('Updated to 0.1.1');
  });
  it('tells a controller outage from a server outage', () => {
    const now = Date.now();
    status.controller = { items: [], error: 'Controller unavailable' };
    const idle = card(status, { controllerSince: now, now });
    expect(idle.view.title).toBe('Controller unavailable');
    expect(idle.view.label).toBe('Check update controller');
    status.request = intent('update');
    const waiting = card(status, { controllerSince: now - 130000, now });
    expect(waiting.view.title).toBe('Controller unavailable');
    expect(card(null, { offlineSince: now, now }).view.title).toBe(
      'Reconnecting to the server',
    );
  });
  it('downloads before preparing and offers a newer release to a queued one', () => {
    // Automatic mode downloads without a queued request.
    status.download = { version: '0.1.1', download: null };
    expect(card(status).view.title).toBe('Downloading the update');
    expect(card(status).progress).toBeNull();
    status.request = intent('update');
    status.download = {
      version: '0.1.1',
      download: { component: 'server', received: 25, total: 100 },
    };
    const downloading = card(status);
    expect(downloading.view.title).toBe('Downloading server update');
    expect(downloading.progress).toBe(25);
    expect(downloading.cancellable).toBe(true);
    status.download = null;
    status.release = { version: '0.1.2', notes: '' };
    status.request.waiting = 'Docker is not reachable';
    const waiting = card(status);
    expect(waiting.view.title).toBe('Waiting to retry the update');
    expect(waiting.view.description).toContain('Docker is not reachable');
    expect(waiting.view.description).toContain('0.1.2 is also available');
    // Validation runs while the server works and can still be cancelled.
    status.controller = {
      items: [{ ...blocked('update'), stage: 'validating' }],
    };
    expect(card(status).cancellable).toBe(true);
    status.controller.items[0].stage = 'snapshotting';
    expect(card(status).cancellable).toBe(false);
  });
  it('explains automatic failures and incompatible releases', () => {
    status.policy.policy = 'automatic';
    status.automatic = {
      version: '0.1.1',
      failures: 2,
      retry_at: 1000,
      error: 'Docker refused to create container: NanoCPUs can not be set',
    };
    const failed = card(status);
    expect(failed.view.title).toBe('Automatic update to 0.1.1 failed');
    expect(failed.view.description).toContain('NanoCPUs');
    expect(failed.view.description).toContain('Automatic updates retry after');
    expect(failed.view.action).toBe('install');
    status.automatic.held = true;
    expect(card(status).view.description).toContain(
      'Automatic updates stopped trying 0.1.1.',
    );
    status.release = null;
    status.observation = {
      checked_at: 1,
      error: null,
      incompatible: {
        version: '0.2.0',
        reason: 'Release cannot migrate the current database',
      },
    };
    expect(card(status).view.title).toBe('0.2.0 needs a manual update');
    status.observation = {
      checked_at: 1,
      error: 'The release channel answered HTTP 404',
    };
    expect(card(status).view.title).toBe('Release check failed');
  });
});
