import type { AppRoute, SettingsSection } from './navigation';

const cached = <T>(load: () => Promise<T>) => {
  let pending: Promise<T> | undefined;
  return () =>
    (pending ??= load().catch((error: unknown) => {
      pending = undefined;
      throw error;
    }));
};
export const pages = {
  home: cached(() => import('./Home.svelte')),
  discover: cached(() => import('./seerr/Discover.svelte')),
  library: cached(() => import('./LibraryView.svelte')),
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
  PasswordSettings: cached(() => import('./PasswordSettings.svelte')),
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

const settingsModules: Record<SettingsSection, (keyof typeof settingsPages)[]> =
  {
    account: ['UserPreferences', 'PasswordSettings'],
    playback: ['PlaybackSettings', 'SegmentSettings'],
    online: ['OnlineAccounts'],
    devices: ['QuickConnect'],
    mpv: ['MpvSettings'],
    connection: ['DesktopUpdatePreferences'],
    server: [
      'ServerDisplayDefaults',
      'ProductVersion',
      'ProductUpdatePreferences',
      'ServerTools',
      'AdminOperations',
    ],
    analysis: ['SegmentSettings'],
    providers: ['OnlineSettings'],
    services: ['ServicesSettings', 'ManagerOwnership'],
    retention: ['RetentionSettings'],
    backups: ['BackupSettings'],
    people: ['UserAdministration'],
    jobs: ['ActivitySettings'],
    audit: ['AuditSettings'],
  };
export async function preloadPage(route: AppRoute) {
  if (route.section === 'Settings') {
    await Promise.all([
      pages.settings(),
      ...settingsModules[route.settings].map((key) => settingsPages[key]()),
    ]);
  } else if (route.section === 'Home') await pages.home();
  else if (route.section === 'Discover') await pages.discover();
  else if (['Movies', 'Shows', 'Music'].includes(route.section))
    await pages.library();
  else if (route.section === 'Playlists') await pages.playlists();
  else if (route.section === 'Statistics') await pages.statistics();
  else await pages.providers();
}
