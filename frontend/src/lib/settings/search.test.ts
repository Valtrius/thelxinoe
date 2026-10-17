import { describe, expect, it } from 'vitest';
import type { User } from '../api';
import { searchSettings, settingsSearchIndex } from './search';
import { settingsGroups } from './sections';

const pagesFor = (role: User['role']) =>
  settingsGroups({ role } as User, {
    youtube: true,
    twitch: true,
    kick: true,
  }).flatMap((group) => group.items);
const admin = pagesFor('admin');
const search = (query: string, pages = admin) =>
  searchSettings(query, pages).map((result) => ({
    page: result.page.id,
    cards: result.cards,
    links: result.links?.map((link) => link.label),
  }));

describe('settings search', () => {
  it('ranks a card title above the same word in another page', () => {
    expect(search('subtitles')).toEqual([
      { page: 'playback', cards: ['Subtitles'], links: undefined },
      { page: 'services', cards: [], links: ['Bazarr'] },
    ]);
  });

  it('shows every card when the query names a page', () => {
    expect(search('backups')).toEqual([
      {
        page: 'backups',
        cards: ['Automatic backups', 'Create a backup', 'Backup archives'],
        links: undefined,
      },
    ]);
  });

  it('requires every word to match within one card', () => {
    expect(search('server timezone')).toEqual([
      { page: 'server', cards: ['Display defaults'], links: undefined },
    ]);
  });

  it('matches word prefixes and plurals', () => {
    expect(search('passkey')[0]).toMatchObject({
      page: 'account',
      cards: ['Sign-in methods'],
    });
    expect(search('radarr')).toEqual([
      { page: 'services', cards: [], links: ['Radarr'] },
    ]);
  });

  it('only searches the pages this person can open', () => {
    expect(search('transcoding', pagesFor('user'))).toEqual([]);
    expect(search('nothing like this')).toEqual([]);
  });
});

// Each settings card has to be in the index, or a search can never show it.
describe('settings search index', () => {
  const sources = import.meta.glob<string>('../**/*.svelte', {
    query: '?raw',
    import: 'default',
    eager: true,
  });
  const literals = (expression: string) =>
    [...expression.matchAll(/'([^']+)'|"([^"]+)"/g)].map(
      (match) => match[1] ?? match[2],
    );
  // The opening tag ends before the card's first child element.
  const cardTitles = Object.entries(sources).flatMap(([file, source]) =>
    source
      .split('<SettingsCard')
      .slice(1)
      .flatMap((tag) => {
        const title = /\btitle=(?:"([^"]*)"|\{([^}]*)\})/.exec(
          tag.slice(0, tag.indexOf('<')),
        );
        if (!title) return [];
        return (title[1] ? [title[1]] : literals(title[2])).map((value) => ({
          file,
          title: value,
        }));
      }),
  );
  const indexed = Object.values(settingsSearchIndex).flatMap((cards) =>
    cards.flatMap((card) => [card.title].flat()),
  );

  it('includes every card title written in a component', () => {
    expect(cardTitles.length).toBeGreaterThan(30);
    expect(cardTitles.filter((card) => !indexed.includes(card.title))).toEqual(
      [],
    );
  });

  it('only lists titles that a component renders', () => {
    const everything = Object.values(sources).join('\n');
    expect(
      indexed.filter(
        (title) =>
          !everything.includes(`'${title}'`) &&
          !everything.includes(`"${title}"`),
      ),
    ).toEqual([]);
  });
});
