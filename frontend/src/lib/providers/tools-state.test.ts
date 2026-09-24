import { get } from 'svelte/store';
import { describe, expect, it, vi } from 'vitest';
vi.mock('./api', () => ({
  normalizeError: (error: unknown) => ({
    message: error instanceof Error ? error.message : String(error),
  }),
}));
import { createToolsState } from './tools-state';
import {
  toolsApi,
  type ToolId,
  type ToolsSnapshot,
  type ToolPreference,
} from './tools-api';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function setup() {
  const snapshot: ToolsSnapshot = {
    tools: (['mpv', 'ytdlp', 'streamlink', 'ffmpeg', 'deno'] as ToolId[]).map(
      (id) => ({
        id,
        preference: {
          source: 'system',
          customPath: null,
          active: null,
          previous: null,
          pinned: false,
          updatePolicy: 'notify',
          channel: 'recommended',
          enabled: false,
          heldVersions: [],
        },
        diagnostic: {
          kind: id,
          detected: true,
          version: '1',
          path: `C:/example/${id}.exe`,
        },
        checkedAt: '2026-09-14T12:00:00Z',
        selectedPath: null,
        importedPaths: [],
        installed: [],
        versions: [],
        inUse: [],
      }),
    ),
    mpv: { source: 'native', directory: null },
    operation: {
      tool: null,
      packageId: null,
      phase: '',
      downloaded: 0,
      total: 0,
      error: null,
    },
    lastChecked: null,
    catalogError: null,
    directory: 'C:/example/tools',
  };
  function save(id: ToolId, preference: ToolPreference) {
    const tool = snapshot.tools.find((item) => item.id === id)!;
    if (
      tool.preference.source !== preference.source ||
      tool.preference.customPath !== preference.customPath
    ) {
      tool.diagnostic = null;
      tool.checkedAt = null;
    }
    tool.preference = preference;
  }
  const api = {
    ...toolsApi,
    get: vi.fn(async () => structuredClone(snapshot)),
    preference: vi.fn(async (id: ToolId, preference: ToolPreference) =>
      save(id, preference),
    ),
    check: vi.fn<(id: ToolId) => Promise<void>>().mockResolvedValue(undefined),
    refresh: vi.fn(async () => {}),
    mpvPreferences: vi.fn(async (preferences: ToolsSnapshot['mpv']) => {
      snapshot.mpv = preferences;
    }),
  };
  const state = createToolsState(api);
  return { state, snapshot, api, save };
}

describe('shared tools state', () => {
  it('checks auto-detection after saving the source and publishes the resolved executable', async () => {
    const { state, snapshot, api, save } = setup();
    snapshot.tools[0].preference.source = 'managed';
    await state.refresh();
    const saving = deferred<void>();
    const checking = deferred<void>();
    api.preference.mockImplementationOnce(async (id, preference) => {
      await saving.promise;
      save(id, preference);
    });
    api.check.mockImplementationOnce(async (id) => {
      expect(id).toBe('mpv');
      expect(snapshot.tools[0].preference.source).toBe('system');
      await checking.promise;
      snapshot.tools[0].diagnostic = {
        kind: 'mpv',
        detected: true,
        path: 'C:/example/detected-mpv.exe',
        version: 'detected version',
      };
      snapshot.tools[0].checkedAt = '2026-09-14T14:00:00Z';
    });
    const switched = state.preference('mpv', { source: 'system' });
    await vi.waitFor(() => expect(api.preference).toHaveBeenCalledTimes(1));
    expect(api.check).not.toHaveBeenCalled();
    expect(get(state).snapshot?.tools[0].preference.source).toBe('system');
    saving.resolve();
    await vi.waitFor(() =>
      expect(api.check).toHaveBeenCalledExactlyOnceWith('mpv'),
    );
    expect(get(state).checking).toEqual(['mpv']);
    expect(get(state).pending.ytdlp).toBeUndefined();
    checking.resolve();
    await expect(switched).resolves.toBe(true);
    expect(get(state).checking).toEqual([]);
    expect(get(state).snapshot?.tools[0].diagnostic?.path).toBe(
      'C:/example/detected-mpv.exe',
    );
  });

  it('does not check when switching to auto-detect fails', async () => {
    const { state, snapshot, api } = setup();
    snapshot.tools[0].preference.source = 'managed';
    await state.refresh();
    api.preference.mockRejectedValueOnce(new Error('Could not save'));
    await expect(state.preference('mpv', { source: 'system' })).resolves.toBe(
      false,
    );
    expect(api.check).not.toHaveBeenCalled();
    expect(get(state).snapshot?.tools[0].preference.source).toBe('managed');
  });

  it('skips a source check when a newer source choice is already queued', async () => {
    const { state, snapshot, api, save } = setup();
    snapshot.tools[0].preference.source = 'managed';
    snapshot.tools[0].preference.customPath = null;
    await state.refresh();
    const saving = deferred<void>();
    api.preference.mockImplementationOnce(async (id, preference) => {
      await saving.promise;
      save(id, preference);
    });
    const automatic = state.preference('mpv', { source: 'system' });
    const custom = state.preference('mpv', {
      source: 'custom',
      customPath: 'C:/example/my-mpv.exe',
    });
    saving.resolve();
    await Promise.all([automatic, custom]);
    expect(api.check).not.toHaveBeenCalled();
    expect(get(state).snapshot?.tools[0].preference.source).toBe('custom');
  });

  it('releases a failed automatic check without disabling other tools', async () => {
    const { state, snapshot, api } = setup();
    snapshot.tools[0].preference.source = 'managed';
    await state.refresh();
    api.check.mockRejectedValueOnce(new Error('Check failed'));
    await expect(state.preference('mpv', { source: 'system' })).resolves.toBe(
      false,
    );
    expect(get(state).checking).toEqual([]);
    expect(get(state).pending).toEqual({});
    expect(get(state).errors).toEqual({ mpv: 'Check failed' });
  });

  it('shares an explicit check queued during an auto-detect source save', async () => {
    const { state, snapshot, api, save } = setup();
    snapshot.tools[0].preference.source = 'managed';
    await state.refresh();
    const saving = deferred<void>();
    const checking = deferred<void>();
    api.preference.mockImplementationOnce(async (id, preference) => {
      await saving.promise;
      save(id, preference);
    });
    api.check.mockImplementationOnce(() => checking.promise);
    const automatic = state.preference('mpv', { source: 'system' });
    const explicit = state.check('mpv');
    saving.resolve();
    await vi.waitFor(() => expect(api.check).toHaveBeenCalledTimes(1));
    expect(get(state).checking).toEqual(['mpv']);
    checking.resolve();
    await Promise.all([automatic, explicit]);
    expect(api.check).toHaveBeenCalledTimes(1);
    expect(get(state).checking).toEqual([]);
  });

  it('saves configuration source choices without waiting for MPV detection', async () => {
    const { state, api } = setup();
    await state.refresh();
    const checking = deferred<void>();
    api.check.mockImplementationOnce(() => checking.promise);
    const check = state.check('mpv');
    await state.mpvPreferences({ source: 'managed', directory: null });
    expect(get(state).snapshot?.mpv.source).toBe('managed');
    expect(get(state).checking).toEqual(['mpv']);
    await state.mpvPreferences({ source: 'native', directory: null });
    expect(get(state).snapshot?.mpv.source).toBe('native');
    expect(get(state).pending.configuration).toBeUndefined();
    checking.resolve();
    await check;
  });

  it('flushes an explicit configuration save even after its editor is unmounted', async () => {
    const { state } = setup();
    const pending = deferred<boolean>();
    state.trackSave(pending.promise);
    let finished = false;
    const closing = state.flushPreferences().then(() => {
      finished = true;
    });
    await Promise.resolve();
    expect(finished).toBe(false);
    pending.resolve(true);
    await closing;
    expect(finished).toBe(true);
  });

  it('does not close while a tracked configuration save reports failure', async () => {
    const { state } = setup();
    const pending = deferred<boolean>();
    state.trackSave(pending.promise);
    const closing = state.flushPreferences();
    pending.resolve(false);
    await expect(closing).rejects.toThrow('could not be saved');
  });

  it('updates only the selected tool optimistically and keeps preferences independent of downloads', async () => {
    const { state, api, save } = setup();
    await state.refresh();
    const saving = deferred<void>();
    const download = deferred<void>();
    api.preference.mockImplementationOnce(async (id, preference) => {
      await saving.promise;
      save(id, preference);
    });
    const install = state.run(
      'ffmpeg',
      () => download.promise,
      'Installing...',
    );
    const changed = state.preference('mpv', {
      source: 'custom',
      customPath: 'C:/example/my-mpv.exe',
    });
    expect(get(state).snapshot?.tools[0].preference.source).toBe('custom');
    expect(get(state).snapshot?.tools[0].diagnostic).toBeNull();
    expect(get(state).snapshot?.tools[1].diagnostic?.detected).toBe(true);
    expect(get(state).pending.ytdlp).toBeUndefined();
    const flushed = state.flushPreferences();
    saving.resolve();
    await expect(changed).resolves.toBe(true);
    await expect(flushed).resolves.toBe(true);
    expect(get(state).pending.ffmpeg).toBe('Installing...');
    expect(api.check).not.toHaveBeenCalled();
    download.resolve();
    await install;
  });

  it('serializes rapid writes to one tool without losing a later choice during reloads', async () => {
    const { state, api, save } = setup();
    await state.refresh();
    const first = deferred<void>();
    api.preference.mockImplementationOnce(async (id, preference) => {
      await first.promise;
      save(id, preference);
    });
    const policy = state.preference('mpv', { updatePolicy: 'manual' });
    const pin = state.preference('mpv', { pinned: true });
    await state.refresh();
    expect(get(state).snapshot?.tools[0].preference).toMatchObject({
      updatePolicy: 'manual',
      pinned: true,
    });
    first.resolve();
    await Promise.all([policy, pin]);
    expect(api.preference).toHaveBeenLastCalledWith(
      'mpv',
      expect.objectContaining({ updatePolicy: 'manual', pinned: true }),
    );
    expect(get(state).snapshot?.tools[0].diagnostic?.detected).toBe(true);
    expect(get(state).pending.mpv).toBeUndefined();
  });

  it.each([
    { pinned: true },
    { updatePolicy: 'manual' as const },
    { source: 'system' as const },
    { source: 'custom' as const, customPath: 'C:/example/my-mpv.exe' },
  ])(
    'saves and flushes %j while the same tool is downloading',
    async (preference) => {
      const { state, api, snapshot } = setup();
      snapshot.tools[0].preference.source = 'managed';
      await state.refresh();
      const download = deferred<void>();
      const install = state.run('mpv', () => download.promise, 'Installing...');
      const changed = state.preference('mpv', preference);
      await vi.waitFor(() => expect(api.preference).toHaveBeenCalledTimes(1));
      await expect(changed).resolves.toBe(true);
      await expect(state.flushPreferences()).resolves.toBe(true);
      expect(snapshot.tools[0].preference).toMatchObject(preference);
      expect(get(state).pending.mpv).toBe('Installing...');
      expect(get(state).snapshot?.tools[0].preference).toMatchObject(
        preference,
      );
      if ('source' in preference && preference.source === 'system')
        expect(api.check).toHaveBeenCalledExactlyOnceWith('mpv');
      else expect(api.check).not.toHaveBeenCalled();
      download.resolve();
      await install;
      expect(get(state).pending.mpv).toBeUndefined();
    },
  );

  it('keeps newer preference drafts and exit saves after a download finishes', async () => {
    const { state, api, save } = setup();
    await state.refresh();
    const download = deferred<void>();
    const saving = deferred<void>();
    api.preference.mockImplementationOnce(async (id, preference) => {
      await saving.promise;
      save(id, preference);
    });
    const install = state.run('mpv', () => download.promise, 'Installing...');
    const first = state.preference('mpv', { pinned: true });
    const second = state.preference('mpv', { updatePolicy: 'manual' });
    await vi.waitFor(() => expect(api.preference).toHaveBeenCalledTimes(1));
    download.resolve();
    await install;
    expect(get(state).pending.mpv).toBe('Saving...');
    expect(get(state).snapshot?.tools[0].preference).toMatchObject({
      pinned: true,
      updatePolicy: 'manual',
    });
    let closed = false;
    const closing = state.flushPreferences().then(() => {
      closed = true;
    });
    await Promise.resolve();
    expect(closed).toBe(false);
    saving.resolve();
    await Promise.all([first, second, closing]);
    expect(closed).toBe(true);
    expect(api.preference).toHaveBeenLastCalledWith(
      'mpv',
      expect.objectContaining({
        pinned: true,
        updatePolicy: 'manual',
      }),
    );
    expect(get(state).pending.mpv).toBeUndefined();
  });

  it('does not clear a failed preference save when a concurrent installation succeeds', async () => {
    const { state, api } = setup();
    await state.refresh();
    const download = deferred<void>();
    const install = state.run('mpv', () => download.promise, 'Installing...');
    api.preference.mockRejectedValueOnce(new Error('Preference write failed'));
    await expect(state.preference('mpv', { pinned: true })).resolves.toBe(
      false,
    );
    expect(get(state).snapshot?.tools[0].preference.pinned).toBe(false);
    download.resolve();
    await install;
    expect(get(state).errors.mpv).toBe('Preference write failed');
    await state.preference('mpv', { pinned: true });
    expect(get(state).errors.mpv).toBeFalsy();
  });

  it('orders a new action after earlier preference writes without blocking later writes', async () => {
    const { state, api, save } = setup();
    await state.refresh();
    const saving = deferred<void>();
    const download = deferred<void>();
    api.preference.mockImplementationOnce(async (id, preference) => {
      await saving.promise;
      save(id, preference);
    });
    const first = state.preference('mpv', { pinned: true });
    const action = vi.fn(() => download.promise);
    const install = state.run('mpv', action, 'Installing...');
    await vi.waitFor(() => expect(api.preference).toHaveBeenCalledTimes(1));
    expect(action).not.toHaveBeenCalled();
    saving.resolve();
    await first;
    await vi.waitFor(() => expect(action).toHaveBeenCalledTimes(1));
    await expect(
      state.preference('mpv', { updatePolicy: 'manual' }),
    ).resolves.toBe(true);
    expect(get(state).pending.mpv).toBe('Installing...');
    download.resolve();
    await install;
  });

  it('waits for both download and preference queues before using the selected executable', async () => {
    const { state, api, save } = setup();
    await state.refresh();
    const download = deferred<void>();
    const saving = deferred<void>();
    api.preference.mockImplementationOnce(async (id, preference) => {
      await saving.promise;
      save(id, preference);
    });
    const install = state.run('mpv', () => download.promise, 'Installing...');
    const changed = state.preference('mpv', { pinned: true });
    let settled = false;
    const waiting = state.settle('mpv').then((success) => {
      settled = success;
    });
    download.resolve();
    await install;
    expect(settled).toBe(false);
    saving.resolve();
    await Promise.all([changed, waiting]);
    expect(settled).toBe(true);
  });

  it('runs independent explicit checks and releases each completed row separately', async () => {
    const { state, api } = setup();
    await state.refresh();
    const mpv = deferred<void>();
    const deno = deferred<void>();
    api.check.mockImplementation((id) =>
      id === 'mpv'
        ? mpv.promise
        : id === 'deno'
          ? deno.promise
          : Promise.resolve(),
    );
    const checks = state.checkAll();
    expect(get(state).checking).toHaveLength(5);
    await vi.waitFor(() =>
      expect(get(state).checking).toEqual(['mpv', 'deno']),
    );
    mpv.resolve();
    await vi.waitFor(() => expect(get(state).checking).toEqual(['deno']));
    deno.resolve();
    await checks;
    expect(get(state).checking).toEqual([]);
    expect(api.check).toHaveBeenCalledTimes(5);
  });

  it('does not let an old fetch overwrite an acknowledged preference', async () => {
    const { state, api, snapshot } = setup();
    await state.refresh();
    const old = structuredClone(snapshot);
    const delayed = deferred<ToolsSnapshot>();
    api.get.mockImplementationOnce(() => delayed.promise);
    const reload = state.refresh();
    const change = state.preference('mpv', { pinned: true });
    await vi.waitFor(() => expect(api.preference).toHaveBeenCalledTimes(1));
    delayed.resolve(old);
    await Promise.all([reload, change]);
    expect(get(state).snapshot?.tools[0].preference.pinned).toBe(true);
  });
});
