import { writable } from 'svelte/store';
import { normalizeError } from './api';
import { LatestRequest } from './latest-request';
import {
  isPlugin,
  toolNames,
  toolsApi,
  type MpvPreferences,
  type ToolId,
  type ToolOperation,
  type ToolPreference,
  type ToolsSnapshot,
} from './tools-api';

export type ToolActionKey = ToolId | 'configuration' | 'updates';
type ToolQueueKey = ToolActionKey | `preference:${ToolId}`;
const toolIds = Object.keys(toolNames) as ToolId[];
const preferenceQueueKey = (id: ToolId): ToolQueueKey => `preference:${id}`;
export interface ToolsState {
  snapshot: ToolsSnapshot | null;
  pending: Partial<Record<ToolActionKey, string>>;
  checking: ToolId[];
  errors: Partial<Record<ToolActionKey, string>>;
  loadError: string;
}

export function createToolsState(api = toolsApi) {
  let snapshot: ToolsSnapshot | null = null;
  let pending: Partial<Record<ToolQueueKey, string>> = {};
  let errors: Partial<Record<ToolQueueKey, string>> = {};
  let loadError = '';
  let mpvDraft: MpvPreferences | null = null;
  const drafts = new Map<ToolId, Partial<ToolPreference>>();
  const queues = new Map<ToolQueueKey, Promise<boolean>>();
  const writes = new Set<Promise<boolean>>();
  const checking = new Set<ToolId>();
  const reloadRequests = new LatestRequest();
  let reloadPending: Promise<void> | null = null;
  let reloadAgain = false;
  let progressRevision = 0;
  const store = writable<ToolsState>({
    snapshot,
    pending,
    checking: [],
    errors,
    loadError,
  });

  function publish() {
    const visible = snapshot && {
      ...snapshot,
      mpv: mpvDraft ?? snapshot.mpv,
      tools: snapshot.tools.map((tool) => {
        const draft = drafts.get(tool.id);
        if (!draft) return tool;
        const preference = { ...tool.preference, ...draft };
        const changed =
          preference.source !== tool.preference.source ||
          preference.customPath !== tool.preference.customPath;
        return {
          ...tool,
          preference,
          diagnostic: changed ? null : tool.diagnostic,
          checkedAt: changed ? null : tool.checkedAt,
          selectedPath: changed
            ? preference.source === 'custom'
              ? preference.customPath
              : null
            : tool.selectedPath,
        };
      }),
    };
    const visiblePending = { ...pending };
    const visibleErrors = { ...errors };
    for (const id of toolIds) {
      const key = preferenceQueueKey(id);
      if (pending[key] && !pending[id]) visiblePending[id] = pending[key];
      const messages = [errors[id], errors[key]].filter(Boolean);
      if (messages.length) visibleErrors[id] = messages.join('\n');
      delete visiblePending[key];
      delete visibleErrors[key];
    }
    store.set({
      snapshot: visible,
      pending: visiblePending,
      checking: [...checking],
      errors: visibleErrors,
      loadError,
    });
  }

  function refresh(): Promise<void> {
    reloadAgain = true;
    reloadRequests.invalidate();
    reloadPending ??= (async () => {
      while (reloadAgain) {
        reloadAgain = false;
        const current = reloadRequests.begin();
        const progressAtStart = progressRevision;
        try {
          const next = await api.get();
          if (current()) {
            if (snapshot && progressAtStart !== progressRevision)
              next.operation = snapshot.operation;
            snapshot = next;
            loadError = '';
            publish();
          }
        } catch (error) {
          if (current()) {
            loadError = normalizeError(error).message;
            publish();
            throw error;
          }
        }
      }
    })().finally(() => {
      reloadPending = null;
    });
    return reloadPending;
  }

  function report(key: ToolActionKey, error: unknown) {
    const queueKey =
      key === 'configuration' || key === 'updates'
        ? key
        : preferenceQueueKey(key);
    errors = { ...errors, [queueKey]: normalizeError(error).message };
    publish();
  }

  function enqueue(
    key: ToolActionKey,
    queueKey: ToolQueueKey,
    action: () => Promise<unknown>,
    label: string,
  ): Promise<boolean> {
    pending = { ...pending, [queueKey]: label };
    errors = { ...errors, [queueKey]: '' };
    publish();
    const previous = queues.get(queueKey) ?? Promise.resolve(true);
    const task = previous
      .then(async () => {
        let success = false;
        try {
          await action();
          errors = { ...errors, [queueKey]: '' };
          success = true;
        } catch (error) {
          errors = { ...errors, [queueKey]: normalizeError(error).message };
          publish();
        }
        try {
          await refresh();
        } catch {
          /* The last known state remains visible with its read error. */
        }
        return success;
      })
      .finally(() => {
        if (queues.get(queueKey) === task) {
          queues.delete(queueKey);
          delete pending[queueKey];
          if (key === 'configuration') mpvDraft = null;
          else if (key !== 'updates' && queueKey !== key) drafts.delete(key);
        }
        publish();
      });
    queues.set(queueKey, task);
    return task;
  }

  function run(
    key: ToolActionKey,
    action: () => Promise<unknown>,
    label = 'Saving...',
  ): Promise<boolean> {
    // Actions wait for earlier preference saves, but later saves never wait for downloads.
    const preferences =
      key !== 'configuration' && key !== 'updates'
        ? queues.get(preferenceQueueKey(key))
        : undefined;
    return enqueue(
      key,
      key,
      async () => {
        if (preferences && !(await preferences))
          throw new Error(
            'Tool preferences could not be saved. Retry the action.',
          );
        await action();
      },
      label,
    );
  }

  function rememberWrite(task: Promise<boolean>) {
    writes.add(task);
    void task.finally(() => writes.delete(task));
    return task;
  }

  function preference(id: ToolId, change: Partial<ToolPreference>) {
    const draft = { ...drafts.get(id), ...change };
    drafts.set(id, draft);
    return rememberWrite(
      enqueue(
        id,
        preferenceQueueKey(id),
        async () => {
          const tool = snapshot?.tools.find((item) => item.id === id);
          if (!tool) throw new Error('Tool settings are not loaded.');
          const next = { ...tool.preference, ...draft };
          await api.preference(id, next);
          const changed =
            next.source !== tool.preference.source ||
            next.customPath !== tool.preference.customPath;
          if (snapshot)
            snapshot = {
              ...snapshot,
              tools: snapshot.tools.map((item) =>
                item.id !== id
                  ? item
                  : {
                      ...item,
                      preference: { ...item.preference, ...draft },
                      diagnostic: changed ? null : item.diagnostic,
                      checkedAt: changed ? null : item.checkedAt,
                      selectedPath: changed
                        ? next.source === 'custom'
                          ? next.customPath
                          : null
                        : item.selectedPath,
                    },
              ),
            };
          // Check the selected executable only after the source is saved. A newer
          // queued source choice must not trigger a check of the old selection.
          if (
            changed &&
            (change.source === 'system' || change.source === 'managed') &&
            !isPlugin(id) &&
            !checking.has(id) &&
            drafts.get(id)?.source === change.source
          ) {
            checking.add(id);
            publish();
            try {
              await api.check(id);
            } finally {
              checking.delete(id);
              publish();
            }
          }
        },
        'Saving...',
      ),
    );
  }

  function mpvPreferences(preferences: MpvPreferences) {
    mpvDraft = preferences;
    return rememberWrite(
      run('configuration', async () => {
        await api.mpvPreferences(preferences);
        if (snapshot) snapshot = { ...snapshot, mpv: preferences };
      }),
    );
  }

  function check(id: ToolId) {
    const key = preferenceQueueKey(id);
    if (checking.has(id)) return queues.get(key) ?? Promise.resolve(true);
    checking.add(id);
    const task = enqueue(id, key, () => api.check(id), 'Checking...');
    void task.finally(() => {
      checking.delete(id);
      publish();
    });
    return task;
  }

  return {
    subscribe: store.subscribe,
    refresh,
    run,
    report,
    clearError(key: ToolActionKey) {
      delete errors[key];
      if (key !== 'configuration' && key !== 'updates')
        delete errors[preferenceQueueKey(key)];
      publish();
    },
    preference,
    mpvPreferences,
    trackSave: rememberWrite,
    check,
    async settle(key: ToolActionKey) {
      const keys: ToolQueueKey[] = [key];
      if (key !== 'configuration' && key !== 'updates')
        keys.push(preferenceQueueKey(key));
      while (keys.some((key) => queues.has(key))) {
        const results = await Promise.all(keys.map((key) => queues.get(key)));
        if (results.some((result) => result === false)) return false;
      }
      return true;
    },
    checkAll: () =>
      Promise.all(
        (snapshot?.tools ?? [])
          .filter((tool) => !isPlugin(tool.id))
          .map((tool) => check(tool.id)),
      ),
    checkUpdates: () =>
      queues.get('updates') ??
      run('updates', api.refresh, 'Checking for updates...'),
    progress(operation: ToolOperation) {
      progressRevision += 1;
      if (snapshot) snapshot = { ...snapshot, operation };
      publish();
    },
    async flushPreferences() {
      while (writes.size) {
        const results = await Promise.all([...writes]);
        if (results.some((saved) => !saved))
          throw new Error(
            'Pending tool or configuration changes could not be saved.',
          );
      }
      return true;
    },
  };
}

export const toolsState = createToolsState();
