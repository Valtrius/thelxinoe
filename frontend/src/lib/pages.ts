import type { AppRoute, SettingsSection } from './navigation';
import { inBackground, pageSettled, trackPageWork } from './page-readiness';
import { missingWebCode } from './api';

const cached = <T>(load: () => Promise<T>) => {
  let pending: Promise<T> | undefined;
  return () =>
    trackPageWork(
      () =>
        (pending ??= load().catch((error: unknown) => {
          pending = undefined;
          // The chunk may belong to a build the server no longer serves.
          void missingWebCode();
          throw error;
        })),
    );
};
export const pages = {
  home: cached(() => import('./Home.svelte')),
  discover: cached(() => import('./seerr/Discover.svelte')),
  library: cached(() => import('./LibraryView.svelte')),
  title: cached(() => import('./title/TitlePage.svelte')),
  playlists: cached(() => import('./Playlists.svelte')),
  statistics: cached(() => import('./statistics/StatisticsView.svelte')),
  providers: cached(() => import('./providers/ProviderView.svelte')),
  settings: cached(() => import('./settings/SettingsPage.svelte')),
};

export const settingsPages = {
  ActivitySettings: cached(() => import('./ActivitySettings.svelte')),
  PlaybackSettings: cached(() => import('./PlaybackSettings.svelte')),
  SegmentSettings: cached(() => import('./SegmentSettings.svelte')),
  ServerDisplayDefaults: cached(() => import('./ServerDisplayDefaults.svelte')),
  LibraryFolders: cached(() => import('./LibraryFolders.svelte')),
  ServerTranscodingSettings: cached(
    () => import('./ServerTranscodingSettings.svelte'),
  ),
  AdminOperations: cached(() => import('./AdminOperations.svelte')),
  BackupSettings: cached(() => import('./BackupSettings.svelte')),
  ProductVersion: cached(() => import('./ProductVersion.svelte')),
  ProductUpdatePreferences: cached(
    () => import('./ProductUpdatePreferences.svelte'),
  ),
  ServerTools: cached(() => import('./ServerTools.svelte')),
  DesktopUpdatePreferences: cached(
    () => import('./DesktopUpdatePreferences.svelte'),
  ),
  UserAdministration: cached(() => import('./UserAdministration.svelte')),
  MpvSettings: cached(() => import('./MpvSettings.svelte')),
  AuditSettings: cached(() => import('./AuditSettings.svelte')),
  UserPreferences: cached(() => import('./UserPreferences.svelte')),
  AuthSettings: cached(() => import('./AuthSettings.svelte')),
  AuthDevices: cached(() => import('./AuthDevices.svelte')),
  OidcSettings: cached(() => import('./OidcSettings.svelte')),
  QuickConnect: cached(() => import('./QuickConnect.svelte')),
  OnlineAccounts: cached(() => import('./OnlineAccounts.svelte')),
  OnlineSettings: cached(() => import('./OnlineSettings.svelte')),
  ServicesSettings: cached(() => import('./ServicesSettings.svelte')),
  ManagerOwnership: cached(() => import('./ManagerOwnership.svelte')),
  RetentionSettings: cached(() => import('./RetentionSettings.svelte')),
};

export const players = {
  native: cached(() => import('./NativePlayer.svelte')),
  music: cached(() => import('./MusicPlayer.svelte')),
  video: cached(() => import('./Player.svelte')),
};
export const providerPages = {
  YoutubeView: cached(
    () => import('./providers/components/youtube/YoutubeView.svelte'),
  ),
  TwitchView: cached(
    () => import('./providers/components/twitch/TwitchView.svelte'),
  ),
  KickView: cached(() => import('./providers/components/kick/KickView.svelte')),
};

// Every module a settings page renders, so they load together instead of one
// after another as the page mounts.
const settingsModules: Record<SettingsSection, (keyof typeof settingsPages)[]> =
  {
    account: ['UserPreferences', 'AuthSettings'],
    playback: ['PlaybackSettings', 'SegmentSettings'],
    online: ['OnlineAccounts'],
    devices: ['QuickConnect', 'AuthDevices'],
    mpv: ['MpvSettings'],
    connection: ['DesktopUpdatePreferences'],
    server: [
      'LibraryFolders',
      'ServerDisplayDefaults',
      'ServerTranscodingSettings',
      'ProductVersion',
      'ProductUpdatePreferences',
      'ServerTools',
      'OidcSettings',
      'AdminOperations',
    ],
    analysis: ['SegmentSettings'],
    providers: ['OnlineSettings'],
    services: ['ServicesSettings'],
    retention: ['RetentionSettings', 'ManagerOwnership'],
    backups: ['BackupSettings'],
    people: ['UserAdministration'],
    jobs: ['ActivitySettings'],
    audit: ['AuditSettings'],
  };
// Once the open settings page shows, the others load one at a time so moving
// between them only waits for their data.
export async function prefetchSettings(
  sections: SettingsSection[],
  signal: AbortSignal,
) {
  await pageSettled();
  for (const key of new Set(
    sections.flatMap((section) => settingsModules[section]),
  )) {
    if (signal.aborted) return;
    await inBackground(() => settingsPages[key]()).catch(() => {});
  }
}
// The browser's video player, with hls.js, stays out of the first load. It
// loads once the first page has had a few seconds to itself, so starting a
// video rarely waits for its code.
export async function prefetchVideoPlayer() {
  await new Promise((resolve) => setTimeout(resolve, 5000));
  await pageSettled();
  await inBackground(() => players.video()).catch(() => {});
}
// The first page's code downloads while the session's data loads.
export function preloadFirstPage(route: AppRoute) {
  void inBackground(() => preloadPage(route)).catch(() => {});
}
export async function preloadPage(route: AppRoute) {
  if (route.section === 'Settings') {
    await Promise.all([
      pages.settings(),
      ...settingsModules[route.settings].map((key) => settingsPages[key]()),
    ]);
  } else if (route.section === 'Home') await pages.home();
  else if (route.section === 'Discover') await pages.discover();
  else if (['Movies', 'Shows'].includes(route.section) && route.item)
    await pages.title();
  else if (['Movies', 'Shows', 'Music'].includes(route.section))
    await pages.library();
  else if (route.section === 'Playlists') await pages.playlists();
  else if (route.section === 'Statistics') await pages.statistics();
  else await pages.providers();
}
