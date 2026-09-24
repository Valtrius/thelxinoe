import { describe, expect, it } from 'vitest';
import type { ToolPackage, ToolView } from './tools-api';
import {
  availableToolUpdate,
  executableSelectionKey,
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
});
