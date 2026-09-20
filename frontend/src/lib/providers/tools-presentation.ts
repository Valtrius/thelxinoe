import {
  isPlugin,
  type MpvPreferences,
  type ToolOperation,
  type ToolView,
} from './tools-api';

export function activeToolPackage(tool: ToolView) {
  return tool.installed.find(
    (item) => item.package.id === tool.preference.active,
  );
}

export function availableToolUpdate(tool: ToolView) {
  const active = activeToolPackage(tool);
  if (
    !active ||
    tool.preference.source !== 'managed' ||
    (isPlugin(tool.id) && !tool.preference.enabled)
  )
    return undefined;
  return tool.versions.find(
    (version) =>
      version.recommended &&
      version.channel === tool.preference.channel &&
      version.id !== active.package.id &&
      Date.parse(version.publishedAt) >=
        Date.parse(active.package.publishedAt) &&
      !tool.preference.heldVersions.includes(version.id),
  );
}

export const displayToolPath = (path: string | null | undefined) =>
  path?.replaceAll('/', '\\') ?? '';
export function executableSelectionKey(tool: ToolView): string | null {
  const path = tool.selectedPath ?? tool.diagnostic?.path;
  const version =
    tool.preference.source === 'managed'
      ? tool.preference.active
      : tool.diagnostic?.version;
  return path ? `${tool.preference.source}:${path}:${version ?? ''}` : null;
}
export const toolOperationActive = (operation: ToolOperation | null) =>
  operation?.phase === 'downloading' || operation?.phase === 'verifying';

export function pluginSelection(
  tool: ToolView,
): 'managed' | 'imported' | 'off' {
  if (!tool.preference.enabled) return 'off';
  return tool.preference.source === 'managed' ? 'managed' : 'imported';
}

export interface ToolSummary {
  source: string;
  version: string;
  updates: string;
  status: string;
  tone: 'success' | 'warning' | 'muted';
}

export function summarizeTool(
  tool: ToolView,
  configurationSource: MpvPreferences['source'],
): ToolSummary {
  const plugin = isPlugin(tool.id);
  const active = activeToolPackage(tool);
  const managed = tool.preference.source === 'managed';
  const mode = pluginSelection(tool);
  const summary: ToolSummary = {
    source: plugin
      ? mode === 'off'
        ? 'Off'
        : managed
          ? 'Thelxinoe'
          : 'Imported local copy'
      : managed
        ? 'Thelxinoe'
        : tool.preference.source === 'custom'
          ? 'Mine · Selected file'
          : 'Mine · Auto-detected',
    version: managed
      ? (active?.package.version ?? '—')
      : plugin
        ? '—'
        : tool.diagnostic?.version?.split(/\r?\n/)[0] || '—',
    updates:
      managed && active && (!plugin || mode !== 'off')
        ? tool.preference.pinned
          ? 'Pinned'
          : { automatic: 'Automatic', notify: 'Notify me', manual: 'Manual' }[
              tool.preference.updatePolicy
            ]
        : plugin
          ? '—'
          : 'Managed outside Thelxinoe',
    status: 'Ready',
    tone: 'success',
  };
  if (plugin) {
    if (configurationSource !== 'managed')
      return { ...summary, status: 'Inactive here', tone: 'muted' };
    if (mode === 'off')
      return { ...summary, version: '—', status: 'Off', tone: 'muted' };
    if (managed && !active)
      return { ...summary, status: 'Not installed', tone: 'muted' };
    if (!managed && !tool.importedPaths.length)
      return { ...summary, status: 'No local copy', tone: 'muted' };
    return { ...summary, status: 'Enabled' };
  }
  if (managed && !active)
    return { ...summary, status: 'Not installed', tone: 'warning' };
  if (!tool.diagnostic)
    return { ...summary, status: 'Not checked', tone: 'muted' };
  if (!tool.diagnostic.detected)
    return { ...summary, status: 'Unavailable', tone: 'warning' };
  if (tool.diagnostic.warning)
    return { ...summary, status: 'Check setup', tone: 'warning' };
  return summary;
}
