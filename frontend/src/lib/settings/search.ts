import type { SettingsSection } from '../navigation';
import { definitions, type ServiceKind } from '../services/feature';
import type { SettingsPageInfo } from './sections';

// Every settings card a search can show, in page order. `title` is the card
// title as rendered; list each title a card switches between. `terms` are its
// row labels and the other words people search for.
type IndexedCard = { title: string | string[]; terms: string };

const autoDeleteTerms =
  'auto-delete retention watched by delete after days grace period storage limit unpinned pins';

export const settingsSearchIndex: Record<
  Exclude<SettingsSection, 'services'>,
  IndexedCard[]
> = {
  account: [
    {
      title: 'Your display preferences',
      terms:
        'profile picture avatar photo display timezone time zone format 12-hour 24-hour clock appearance theme dark light animation duration motion poster video density fade watched',
    },
    {
      title: 'Sign-in methods',
      terms:
        'passkeys authenticator app totp two-factor 2fa mfa password security login',
    },
    {
      title: ['Change password', 'Set password'],
      terms: 'current new password sign out revoke client passwords',
    },
  ],
  playback: [
    {
      title: 'Video and audio',
      terms:
        'default quality resolution bitrate audio language replaygain volume normalization album track',
    },
    {
      title: 'Subtitles',
      terms: 'captions subtitle language forced',
    },
    {
      title: 'Intro and credit skipping',
      terms:
        'intro recap credits preview skip segments automatically ask ignore',
    },
  ],
  online: [
    {
      title: 'Connected accounts',
      terms:
        'youtube twitch kick connect sign in synchronization sync feed channels subscriptions',
    },
    {
      title: 'Imported data',
      terms: 'delete youtube twitch kick data history channels',
    },
  ],
  devices: [
    {
      title: 'Your devices',
      terms: 'sessions browsers desktops sign out revoke this device',
    },
    {
      title: 'Connect a TV',
      terms: 'quick connect tv code media client jellyfin',
    },
    {
      title: 'Remembered devices',
      terms: 'forget remember trusted verification',
    },
    {
      title: 'Client passwords',
      terms: 'app passwords media clients jellyfin tv totp create revoke',
    },
  ],
  mpv: [
    {
      title: 'Player and tools',
      terms:
        'mpv player tools install update configuration mpv.conf input.conf shortcuts key bindings yt-dlp ytdlp ffmpeg streamlink',
    },
  ],
  connection: [
    {
      title: 'Desktop',
      terms:
        'desktop app version updates automatic notify server address switch change server',
    },
  ],
  server: [
    {
      title: 'Server status',
      terms: 'version docker controller cache space health',
    },
    {
      title: 'Display defaults',
      terms: 'default timezone time zone format 12-hour 24-hour clock',
    },
    {
      title: 'Updates and maintenance',
      terms:
        'server updates automatic install releases maintenance window schedule',
    },
    {
      title: 'Transcoding',
      terms: 'allow video audio transcoding',
    },
    {
      title: 'Tools',
      terms:
        'bundled binaries ffmpeg yt-dlp ytdlp streamlink versions pinned check updates',
    },
    {
      title: 'OpenID Connect',
      terms:
        'oidc sso single sign-on identity provider discovery url client id secret issuer redirect uri sign-in button',
    },
    {
      title: 'Administration overview',
      terms:
        'health storage disk space integrations downloads indexers active playback transcodes recent failures',
    },
    {
      title: 'Diagnostics',
      terms: 'export redacted diagnostics support logs',
    },
    {
      title: 'All devices',
      terms: 'sessions every user revoke access',
    },
  ],
  analysis: [
    {
      title: 'Detection',
      terms:
        'intro credits detect recurring audio locally fingerprint theintrodb timestamps refresh analysis',
    },
    {
      title: 'Analysis queue',
      terms: 'intro credits episodes queue state',
    },
  ],
  providers: [
    {
      title: 'YouTube',
      terms:
        'google client id secret oauth redirect uri application daily api budget quota public video network allow downloads',
    },
    {
      title: 'Twitch',
      terms: 'client id oauth application integration',
    },
    {
      title: 'Kick',
      terms: 'client id secret oauth application integration',
    },
  ],
  retention: [
    { title: 'Movies', terms: autoDeleteTerms },
    {
      title: 'TV seasons',
      terms: `${autoDeleteTerms} shows series episodes exclude specials season 0`,
    },
    {
      title: 'Video downloads',
      terms: `${autoDeleteTerms} youtube twitch kick videos delete downloads`,
    },
    {
      title: 'Library files',
      terms: 'file ownership reconcile permissions recent operations',
    },
  ],
  backups: [
    {
      title: 'Automatic backups',
      terms:
        'schedule passphrase keep retention maintenance window last automatic backup',
    },
    {
      title: 'Create a backup',
      terms: 'passphrase encryption encrypted manual',
    },
    {
      title: 'Backup archives',
      terms: 'restore download delete archives components',
    },
  ],
  people: [
    {
      title: 'Add user',
      terms: 'create account username password role administrator',
    },
    {
      title: 'User access',
      terms:
        'users roles administrator permissions automatic request approval seerr recovery link reset password delete user',
    },
  ],
  jobs: [
    {
      title: 'Background jobs',
      terms: 'jobs running queued failed completed state',
    },
  ],
  audit: [
    {
      // The audit view switches between these three cards.
      title: [
        'Administrative activity',
        'Scheduled deletions',
        'Deletion history',
      ],
      terms:
        'administrative actions history log users deletions scheduled keep delete now',
    },
  ],
};

const serviceTerms: Record<ServiceKind, string> = {
  seerr: 'requests discover approval jellyseerr overseerr',
  recyclarr: 'trash guides quality profiles custom formats',
  radarr: 'movies films quality profile root folder',
  sonarr: 'tv shows series episodes quality profile root folder',
  lidarr: 'music albums artists',
  bazarr: 'subtitles',
  prowlarr: 'indexers trackers torznab newznab',
  nzbget: 'usenet downloads download client',
};

export type ServiceLink = {
  service?: ServiceKind;
  label: string;
  description: string;
};
export type SettingsSearchResult = {
  page: SettingsPageInfo;
  score: number;
  // Titles of the matching cards; the page keeps its own card order.
  cards: string[];
  // The media services page shows one service at a time, so a search links
  // into it instead of rendering it.
  links?: ServiceLink[];
};

const words = (text: string) =>
  text
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);
// Plural queries still find singular labels.
const stem = (token: string) =>
  token.length > 3 && token.endsWith('s') ? token.slice(0, -1) : token;

type Entry = { heading: string[]; terms: string[] };
const cardEntries = Object.fromEntries(
  Object.entries(settingsSearchIndex).map(([section, cards]) => [
    section,
    cards.map((card) => {
      const titles = [card.title].flat();
      return {
        titles,
        heading: words(titles.join(' ')),
        terms: words(card.terms),
      };
    }),
  ]),
) as Record<string, (Entry & { titles: string[] })[]>;
const serviceEntries: (Entry & { link: ServiceLink })[] = [
  {
    link: {
      label: 'All services',
      description: 'Install, connect and update managed containers.',
    },
    heading: words('All services'),
    terms: words('docker containers managed install connect updates'),
  },
  ...definitions.map((definition) => ({
    link: {
      service: definition.kind,
      label: definition.label,
      description: definition.description,
    },
    heading: words(definition.label),
    terms: words(`${definition.description} ${serviceTerms[definition.kind]}`),
  })),
];

// Words people can see outrank words they cannot; the page name counts as a
// heading for every card on it. Every token has to match somewhere.
function score(tokens: string[], entry: Entry, page: string[]) {
  const heading = [...entry.heading, ...page];
  let total = 0;
  for (const token of tokens) {
    const weight = heading.includes(token)
      ? 4
      : heading.some((word) => word.startsWith(token))
        ? 3
        : entry.terms.some((word) => word.startsWith(token))
          ? 2
          : 0;
    if (!weight) return 0;
    total += weight;
  }
  return total;
}

// Pages with a match, best first; ties keep navigation order.
export function searchSettings(
  query: string,
  pages: SettingsPageInfo[],
): SettingsSearchResult[] {
  const tokens = [...new Set(words(query).map(stem))];
  if (!tokens.length) return [];
  const results: SettingsSearchResult[] = [];
  for (const page of pages) {
    const label = words(page.label);
    if (page.id === 'services') {
      const links = serviceEntries
        .map((entry) => ({ entry, score: score(tokens, entry, label) }))
        .filter((match) => match.score)
        .sort((a, b) => b.score - a.score);
      if (links.length)
        results.push({
          page,
          score: links[0].score,
          cards: [],
          links: links.map((match) => match.entry.link),
        });
      continue;
    }
    let best = 0;
    const cards: string[] = [];
    for (const entry of cardEntries[page.id] ?? []) {
      const value = score(tokens, entry, label);
      if (!value) continue;
      best = Math.max(best, value);
      cards.push(...entry.titles);
    }
    if (best) results.push({ page, score: best, cards });
  }
  return results.sort((a, b) => b.score - a.score);
}
