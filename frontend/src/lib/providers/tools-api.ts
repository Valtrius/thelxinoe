import { invoke as call } from '@tauri-apps/api/core';
import type { ExecutableDiagnostic } from './types';
export type ToolId =
  | 'mpv'
  | 'ytdlp'
  | 'streamlink'
  | 'ffmpeg'
  | 'deno'
  | 'uosc'
  | 'thumbfast'
  | 'sub-select';
export type ToolSource = 'system' | 'custom' | 'managed';
export interface ToolPreference {
  source: ToolSource;
  customPath: string | null;
  active: string | null;
  previous: string | null;
  pinned: boolean;
  updatePolicy: 'automatic' | 'notify' | 'manual';
  channel: string;
  enabled: boolean;
  heldVersions: string[];
}
export interface ToolPackage {
  id: string;
  tool: ToolId;
  version: string;
  channel: string;
  recommended: boolean;
  url: string;
  sha256: string;
  size: number;
  format: string;
  entry: string;
  provider: string;
  homepage: string;
  sourceUrl: string;
  license: string;
  publishedAt: string;
  dependencies: ToolId[];
}
export interface InstalledPackage {
  package: ToolPackage;
  installedAt: string;
  directory: string;
  versionOutput: string;
}
export interface ToolView {
  id: ToolId;
  preference: ToolPreference;
  diagnostic: ExecutableDiagnostic | null;
  checkedAt: string | null;
  selectedPath: string | null;
  importedPaths: string[];
  versions: ToolPackage[];
  installed: InstalledPackage[];
  inUse: string[];
}
export interface MpvPreferences {
  source: 'native' | 'managed' | 'directory';
  directory: string | null;
}
export interface ToolOperation {
  tool: ToolId | null;
  packageId: string | null;
  phase: string;
  downloaded: number;
  total: number;
  error: string | null;
}
export interface ToolsSnapshot {
  tools: ToolView[];
  mpv: MpvPreferences;
  operation: ToolOperation;
  lastChecked: string | null;
  catalogError: string | null;
  directory: string;
}
export interface ToolUpdate {
  tool: ToolId;
  packageId: string;
  version: string;
  currentVersion: string;
}
export interface ConfigDocument {
  name: string;
  text: string;
  revision: string;
  hasBackup: boolean;
}
export interface MpvOption {
  name: string;
  type: string;
  'default-value'?: unknown;
  min?: number;
  max?: number;
  choices?: string[];
  'expects-file'?: boolean;
}
export interface MpvSchema {
  executable: string;
  version: string;
  options: MpvOption[];
}
export const toolNames: Record<ToolId, string> = {
  mpv: 'MPV',
  ytdlp: 'yt-dlp',
  streamlink: 'Streamlink',
  ffmpeg: 'FFmpeg + FFprobe',
  deno: 'Deno',
  uosc: 'uosc',
  thumbfast: 'thumbfast',
  'sub-select': 'sub-select',
};
export const isPlugin = (id: ToolId) =>
  ['uosc', 'thumbfast', 'sub-select'].includes(id);
export const toolsApi = {
  get: () => call<ToolsSnapshot>('tools_get'),
  check: (tool: ToolId) => call<void>('tools_check', { tool }),
  updates: () => call<ToolUpdate[]>('tools_updates'),
  update: (packageId: string) => call<void>('tools_update', { packageId }),
  preference: (tool: ToolId, preference: ToolPreference) =>
    call<void>('tools_set_preference', { tool, preference }),
  refresh: () => call<void>('tools_refresh'),
  install: (packageId: string) => call<void>('tools_install', { packageId }),
  activate: (tool: ToolId, packageId: string) =>
    call<void>('tools_activate', { tool, packageId }),
  rollback: (tool: ToolId) => call<void>('tools_rollback', { tool }),
  remove: (packageId: string) => call<void>('tools_remove', { packageId }),
  repair: (packageId: string) => call<void>('tools_repair', { packageId }),
  mpvPreferences: (preferences: MpvPreferences) =>
    call<void>('mpv_set_preferences', { preferences }),
  files: () => call<string[]>('mpv_config_files'),
  read: (name: string) => call<ConfigDocument>('mpv_config_read', { name }),
  save: (name: string, text: string, revision: string) =>
    call<ConfigDocument>('mpv_config_save', { name, text, revision }),
  restore: (name: string, revision: string) =>
    call<ConfigDocument>('mpv_config_restore', { name, revision }),
  importConfig: (directory: string) =>
    call<void>('mpv_config_import', { directory }),
  importPlugin: (path: string) => call<void>('mpv_plugin_import', { path }),
  openConfigurationDirectory: () =>
    call<void>('mpv_open_configuration_directory'),
  testConfiguration: (clean: boolean) =>
    call<string>('mpv_test_configuration', { clean }),
  schema: () => call<MpvSchema>('mpv_options'),
};
