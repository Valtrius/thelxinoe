import type { YoutubeCardShortcut } from './providers/types';
import { get, writable } from 'svelte/store';
import { api } from './api';
export type Theme = 'light' | 'system' | 'dark';
export type Appearance = {
  provider_preferences: Record<string, string>;
  audio_volume: number;
  player_height: number | null;
  youtube_card_shortcuts: YoutubeCardShortcut[];
  theme: Theme;
  sidebar_collapsed: boolean;
  card_columns: number;
  fade_watched: boolean;
  thumbnail_fit: 'contain' | 'cover';
};
const defaults: Appearance = {
  provider_preferences: {},
  audio_volume: 1,
  player_height: null,
  youtube_card_shortcuts: [],
  theme: 'system',
  sidebar_collapsed: false,
  card_columns: 6,
  fade_watched: true,
  thumbnail_fit: 'contain',
};
const cachedTheme = localStorage.getItem('thelxinoe-theme');
export const appearance = writable<Appearance>({
  ...defaults,
  theme: ['light', 'dark', 'system'].includes(cachedTheme ?? '')
    ? (cachedTheme as Theme)
    : 'system',
});
export const appearanceError = writable('');
let owner = '';
let generation = 0;
let acceptedRevision = 0;
let pending = Promise.resolve();
const localChanges = new Map<number, Partial<Appearance>>();
let changeId = 0;
function merge(value: Appearance, change: Partial<Appearance>): Appearance {
  return {
    ...value,
    ...change,
    provider_preferences: {
      ...value.provider_preferences,
      ...change.provider_preferences,
    },
  };
}
export function acceptAppearance(value: Appearance) {
  acceptedRevision++;
  let next = { ...defaults, ...value };
  for (const change of localChanges.values()) next = merge(next, change);
  appearance.set(next);
}
function updateBrowserColor() {
  document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute(
      'content',
      getComputedStyle(document.documentElement)
        .getPropertyValue('--background')
        .trim(),
    );
}
matchMedia('(prefers-color-scheme: dark)').addEventListener(
  'change',
  updateBrowserColor,
);
appearance.subscribe((value) => {
  document.documentElement.dataset.theme = value.theme;
  localStorage.setItem('thelxinoe-theme', value.theme);
  updateBrowserColor();
});
export async function loadAppearance(userId: string) {
  owner = userId;
  const current = ++generation;
  try {
    const value = await api<Appearance>('/me/appearance');
    if (current === generation) {
      acceptAppearance(value);
      appearanceError.set('');
    }
  } catch {
    if (current === generation) {
      appearance.set({ ...defaults, theme: get(appearance).theme });
      appearanceError.set('Appearance preferences could not be loaded.');
    }
  }
}
export async function refreshAppearance(userId: string) {
  if (owner !== userId) return;
  const current = generation;
  const revision = acceptedRevision;
  try {
    const value = await api<Appearance>('/me/appearance');
    if (
      current === generation &&
      owner === userId &&
      revision === acceptedRevision
    ) {
      acceptAppearance(value);
      appearanceError.set('');
    }
  } catch {
    if (current === generation && owner === userId)
      appearanceError.set('Appearance preferences could not be loaded.');
  }
}
export function resetAppearance() {
  owner = '';
  generation++;
  localChanges.clear();
}
export function updateAppearance(change: Partial<Appearance>) {
  appearance.update((value) => merge(value, change));
  if (!owner) return;
  const current = generation;
  const id = ++changeId;
  localChanges.set(id, change);
  pending = pending.then(async () => {
    if (current !== generation) return;
    try {
      await api('/me/appearance', 'PATCH', change);
      appearanceError.set('');
    } catch {
      if (current === generation)
        appearanceError.set(
          'Your appearance changed here, but could not be saved to the server.',
        );
    } finally {
      localChanges.delete(id);
    }
  });
}
