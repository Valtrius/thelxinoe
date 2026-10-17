import {
  Activity,
  AppWindow,
  AudioLines,
  ArchiveRestore,
  Boxes,
  Clapperboard,
  Hourglass,
  Laptop,
  Link,
  MonitorSmartphone,
  Play,
  ScrollText,
  Server,
  UserRound,
  UsersRound,
} from '@lucide/svelte';
import { desktop, type User } from '../api';
import type { SettingsSection } from '../navigation';
import type { ProviderAvailability } from '../providers/availability';

export type SettingsPageInfo = {
  id: SettingsSection;
  label: string;
  icon: typeof UserRound;
  // One line under the title explains what each page is for.
  description: string;
};
export type SettingsGroup = { label: string; items: SettingsPageInfo[] };

const personal: SettingsPageInfo[] = [
  {
    id: 'account',
    label: 'Account',
    icon: UserRound,
    description:
      'Your profile, display preferences, appearance and how you sign in.',
  },
  {
    id: 'playback',
    label: 'Playback',
    icon: Play,
    description:
      'Defaults for new playback sessions. Choices made inside the player override them for that title.',
  },
  {
    id: 'online',
    label: 'Online accounts',
    icon: Link,
    description:
      'Accounts connected to this server for YouTube, Twitch and Kick.',
  },
  {
    id: 'devices',
    label: 'Devices',
    icon: MonitorSmartphone,
    description:
      'Browsers, desktops and media clients that can use your account.',
  },
];
const native: SettingsPageInfo[] = [
  {
    id: 'mpv',
    label: 'MPV',
    icon: Clapperboard,
    description:
      'The desktop player, its bundled tools and their configuration.',
  },
  {
    id: 'connection',
    label: 'Desktop',
    icon: Laptop,
    description: 'This desktop app and the server it connects to.',
  },
];
const admin: SettingsPageInfo[] = [
  {
    id: 'server',
    label: 'Server',
    icon: Server,
    description: 'Health, defaults and maintenance for this Thelxinoe server.',
  },
  {
    id: 'analysis',
    label: 'Episode analysis',
    icon: AudioLines,
    description: 'Finds intro and credit segments so viewers can skip them.',
  },
  {
    id: 'providers',
    label: 'Provider applications',
    icon: AppWindow,
    description:
      'Server-wide applications that let people connect their own YouTube, Twitch and Kick accounts.',
  },
  {
    id: 'services',
    label: 'Media services',
    icon: Boxes,
    description:
      'Managed containers and connections for requests, downloads and library automation.',
  },
  {
    id: 'retention',
    label: 'Auto-delete',
    icon: Hourglass,
    description:
      'Free up space by removing media once the right people have watched it.',
  },
  {
    id: 'backups',
    label: 'Backups',
    icon: ArchiveRestore,
    description:
      'Encrypted archives of server state, credentials, deployment information and managed-service appdata.',
  },
  {
    id: 'people',
    label: 'People',
    icon: UsersRound,
    description: 'Accounts on this server and what they can do.',
  },
  {
    id: 'jobs',
    label: 'Activity',
    icon: Activity,
    description: 'Background jobs running on this server.',
  },
  {
    id: 'audit',
    label: 'Audit',
    icon: ScrollText,
    description:
      'History of administrative actions and media deletions on this server.',
  },
];

// The settings pages this person can open, grouped as the navigation shows them.
export function settingsGroups(
  user: User,
  providers: ProviderAvailability,
): SettingsGroup[] {
  return [
    {
      label: 'Personal',
      items: [
        ...personal.filter(
          (item) =>
            item.id !== 'online' || Object.values(providers).some(Boolean),
        ),
        ...(desktop ? native : []),
      ],
    },
    ...(user.role === 'admin'
      ? [{ label: 'Administration', items: admin }]
      : []),
  ];
}
