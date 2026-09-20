import { describe, expect, it } from 'vitest';
import type { ToolPackage, ToolView } from './tools-api';
import {
  availableToolUpdate,
  executableSelectionKey,
  summarizeTool,
} from './tools-presentation';

const current: ToolPackage = {
  id: 'mpv-current',
  tool: 'mpv',
  version: '1',
  channel: 'stable',
  recommended: false,
  url: '',
  sha256: '',
  size: 1,
  format: 'zip',
  entry: 'mpv.exe',
  provider: '',
  homepage: '',
  sourceUrl: '',
  license: '',
  publishedAt: '2026-01-01',
  dependencies: [],
};
const newer: ToolPackage = {
  ...current,
  id: 'mpv-new',
  version: '2',
  recommended: true,
  publishedAt: '2026-02-01',
};
function tool(): ToolView {
  return {
    id: 'mpv',
    preference: {
      source: 'managed',
      customPath: null,
      active: current.id,
      previous: null,
      pinned: false,
      updatePolicy: 'notify',
      channel: 'stable',
      enabled: true,
      heldVersions: [],
    },
    diagnostic: { kind: 'mpv', detected: true, version: 'mpv 1' },
    checkedAt: '2026-09-14T12:00:00Z',
    selectedPath: null,
    importedPaths: [],
    versions: [current, newer],
    installed: [
      {
        package: current,
        installedAt: '',
        directory: '',
        versionOutput: 'mpv 1',
      },
    ],
    inUse: [],
  };
}

describe('tool inventory summaries', () => {
  it.each(['system', 'custom'] as const)(
    'reports the selected %s executable instead of an unused managed package',
    (source) => {
      const item = tool();
      item.preference.source = source;
      expect(availableToolUpdate(item)).toBeUndefined();
      item.diagnostic = {
        kind: 'mpv',
        detected: true,
        version: 'mpv external\nBuild details',
      };
      expect(summarizeTool(item, 'managed')).toMatchObject({
        version: 'mpv external',
        status: 'Ready',
        updates: 'Managed outside Thelxinoe',
        source:
          source === 'custom' ? 'Mine · Selected file' : 'Mine · Auto-detected',
      });
      item.diagnostic.detected = false;
      item.diagnostic.version = null;
      expect(summarizeTool(item, 'managed')).toMatchObject({
        version: '—',
        status: 'Unavailable',
      });
    },
  );

  it('prioritizes an unusable executable or warning over an available update', () => {
    const item = tool();
    item.diagnostic = { kind: 'mpv', detected: false };
    expect(summarizeTool(item, 'managed')).toMatchObject({
      status: 'Unavailable',
    });
    item.diagnostic = {
      kind: 'mpv',
      detected: true,
      warning: 'Selected installation is incomplete',
    };
    expect(summarizeTool(item, 'managed')).toMatchObject({
      status: 'Check setup',
    });
  });

  it('distinguishes missing optional plugins from missing selected executables', () => {
    const item = tool();
    item.installed = [];
    expect(summarizeTool(item, 'managed')).toMatchObject({
      status: 'Not installed',
      version: '—',
    });
    item.id = 'uosc';
    item.diagnostic = null;
    expect(summarizeTool(item, 'managed')).toMatchObject({
      status: 'Not installed',
    });
  });

  it('distinguishes enabled plugins from plugins inactive under a different configuration source', () => {
    const item = tool();
    item.id = 'uosc';
    item.versions = [];
    item.diagnostic = null;
    expect(summarizeTool(item, 'managed').status).toBe('Enabled');
    expect(summarizeTool(item, 'native')).toMatchObject({
      status: 'Inactive here',
    });
    expect(summarizeTool(item, 'directory').status).toBe('Inactive here');
    item.preference.enabled = false;
    expect(summarizeTool(item, 'managed').status).toBe('Off');
  });

  it('respects held versions, release channel, and publication ordering', () => {
    const item = tool();
    expect(availableToolUpdate(item)).toBeDefined();
    expect(availableToolUpdate(item)).toEqual(newer);
    item.preference.heldVersions = [newer.id];
    expect(availableToolUpdate(item)).toBeUndefined();
    item.preference.heldVersions = [];
    item.versions = [{ ...newer, channel: 'nightly' }];
    expect(availableToolUpdate(item)).toBeUndefined();
    item.versions = [{ ...newer, publishedAt: '2025-12-01' }];
    expect(availableToolUpdate(item)).toBeUndefined();
  });

  it.each(['automatic', 'notify', 'manual'] as const)(
    'shows catalog updates under %s policy, including when pinned',
    (policy) => {
      const item = tool();
      item.preference.updatePolicy = policy;
      item.preference.pinned = true;
      expect(summarizeTool(item, 'managed')).toMatchObject({
        status: 'Ready',
        updates: 'Pinned',
      });
    },
  );

  it('does not reuse a ready status for an unchecked selection', () => {
    const item = tool();
    item.diagnostic = null;
    expect(summarizeTool(item, 'managed').status).toBe('Not checked');
  });

  it('uses the selected managed executable path even before a diagnostic is cached', () => {
    const item = tool();
    item.diagnostic = null;
    item.selectedPath = 'C:/example/managed/mpv.exe';
    expect(executableSelectionKey(item)).toBe(
      'managed:C:/example/managed/mpv.exe:mpv-current',
    );
  });

  it.each(['system', 'custom'] as const)(
    'keys a %s executable by its checked version, not a retained managed package',
    (source) => {
      const item = tool();
      item.preference.source = source;
      item.selectedPath = source === 'custom' ? 'C:/example/mpv.exe' : null;
      item.diagnostic = {
        kind: 'mpv',
        detected: true,
        path: 'C:/example/mpv.exe',
        version: 'mpv 1',
      };
      const previous = executableSelectionKey(item);
      expect(previous).toBe(`${source}:C:/example/mpv.exe:mpv 1`);
      item.diagnostic.version = 'mpv 2';
      expect(executableSelectionKey(item)).toBe(
        `${source}:C:/example/mpv.exe:mpv 2`,
      );
      expect(executableSelectionKey(item)).not.toBe(previous);
      item.preference.active = newer.id;
      expect(executableSelectionKey(item)).toBe(
        `${source}:C:/example/mpv.exe:mpv 2`,
      );
    },
  );

  it('identifies imported plugins and does not offer managed updates for them', () => {
    const item = tool();
    item.id = 'uosc';
    item.preference.source = 'custom';
    item.importedPaths = ['C:/example/config/scripts/uosc'];
    expect(summarizeTool(item, 'managed')).toMatchObject({
      source: 'Imported local copy',
      status: 'Enabled',
      version: '—',
      updates: '—',
    });
    expect(availableToolUpdate(item)).toBeUndefined();
    item.preference.enabled = false;
    expect(summarizeTool(item, 'managed').status).toBe('Off');
    item.preference.source = 'managed';
    expect(availableToolUpdate(item)).toBeUndefined();
  });
});
