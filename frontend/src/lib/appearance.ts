import type { YoutubeCardShortcut } from './providers/types';
import { get, writable } from 'svelte/store';
import { api } from './api';
export type Theme = 'light' | 'system' | 'dark';
export type Appearance = {
  youtube_card_shortcuts: YoutubeCardShortcut[];
  theme: Theme;
  sidebar_collapsed: boolean;
  card_columns: number;
  fade_watched: boolean;
  thumbnail_fit: 'contain' | 'cover';
};
const defaults: Appearance = {
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
let pending = Promise.resolve();
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
      appearance.set(value);
      appearanceError.set('');
    }
  } catch {
    if (current === generation) {
      appearance.set({ ...defaults, theme: get(appearance).theme });
      appearanceError.set('Appearance preferences could not be loaded.');
    }
  }
}
export function resetAppearance() {
  owner = '';
  generation++;
}
export function updateAppearance(change: Partial<Appearance>) {
  appearance.update((value) => ({ ...value, ...change }));
  if (!owner) return;
  const current = generation;
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
    }
  });
}
