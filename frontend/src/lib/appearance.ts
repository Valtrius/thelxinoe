import type { YoutubeCardShortcut } from './providers/types';
import { get, writable } from 'svelte/store';
import { api } from './api';
import { isServerUpdateInterruption } from './server-updates';
import { defaultAnimationDuration, setAnimationDuration } from './motion';
export type Theme = 'light' | 'system' | 'dark';
export type Appearance = {
  provider_preferences: Record<string, string>;
  audio_volume: number;
  player_height: number | null;
  youtube_card_shortcuts: YoutubeCardShortcut[];
  theme: Theme;
  sidebar_collapsed: boolean;
  poster_columns: number;
  video_columns: number;
  animation_duration_ms: number;
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
  poster_columns: 6,
  video_columns: 6,
  animation_duration_ms: defaultAnimationDuration,
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
export const videoMuted = writable(false);
let owner = '';
let motionLoaded = false;
let generation = 0;
let acceptedRevision = 0;
let pending = Promise.resolve();
const localChanges = new Map<number, Partial<Appearance>>();
const failedChanges = new Set<number>();
let loadFailed = false;
let initialChanges: Partial<Appearance>[] | undefined;
// A change after a pause saves at once. Changes that follow it merge into one
// save once input pauses, so a slider drag saves where it starts and stops.
let queued:
  { current: number; id: number; change: Partial<Appearance> } | undefined;
let saveTimer: ReturnType<typeof setTimeout> | undefined;
const savePause = 250;
let changeId = 0;
videoMuted.subscribe((muted) => {
  if (owner)
    localStorage.setItem(`thelxinoe-video-muted:${owner}`, String(muted));
});
function merge<T extends Partial<Appearance>>(
  value: T,
  change: Partial<Appearance>,
): T {
  return {
    ...value,
    ...change,
    ...(change.provider_preferences && {
      provider_preferences: {
        ...value.provider_preferences,
        ...change.provider_preferences,
      },
    }),
  };
}
function updateError() {
  appearanceError.set(
    failedChanges.size
      ? 'Your appearance changed here, but could not be saved to the server.'
      : loadFailed
        ? 'Appearance preferences could not be loaded.'
        : '',
  );
}
function acknowledgeSavedChange(id: number, change: Partial<Appearance>) {
  for (const failedId of failedChanges) {
    if (failedId >= id) continue;
    const remaining = { ...localChanges.get(failedId) };
    for (const key of Object.keys(change) as (keyof Appearance)[]) {
      if (key === 'provider_preferences') {
        const providers = { ...remaining.provider_preferences };
        for (const provider of Object.keys(change.provider_preferences ?? {}))
          delete providers[provider];
        if (Object.keys(providers).length)
          remaining.provider_preferences = providers;
        else delete remaining.provider_preferences;
      } else delete remaining[key];
    }
    if (Object.keys(remaining).length) localChanges.set(failedId, remaining);
    else {
      localChanges.delete(failedId);
      failedChanges.delete(failedId);
    }
  }
}
export function acceptAppearance(value: Appearance) {
  motionLoaded = true;
  acceptedRevision++;
  let next = { ...defaults, ...value };
  for (const change of initialChanges ?? []) next = merge(next, change);
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
  setAnimationDuration(motionLoaded ? value.animation_duration_ms : 0);
  document.documentElement.dataset.theme = value.theme;
  localStorage.setItem('thelxinoe-theme', value.theme);
  updateBrowserColor();
});
export async function loadAppearance(userId: string) {
  localChanges.clear();
  failedChanges.clear();
  queued = undefined;
  loadFailed = false;
  updateError();
  initialChanges = [];
  motionLoaded = false;
  appearance.set({ ...defaults, theme: get(appearance).theme });
  owner = userId;
  videoMuted.set(
    localStorage.getItem(`thelxinoe-video-muted:${userId}`) === 'true',
  );
  const current = ++generation;
  try {
    const value = await api<Appearance>('/me/appearance');
    if (current === generation) {
      acceptAppearance(value);
      loadFailed = false;
      updateError();
    }
  } catch (error) {
    if (isServerUpdateInterruption(error)) return;
    if (current === generation) {
      loadFailed = true;
      updateError();
    }
  } finally {
    if (current === generation) initialChanges = undefined;
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
      loadFailed = false;
      updateError();
    }
  } catch (error) {
    if (isServerUpdateInterruption(error)) return;
    if (current === generation && owner === userId) {
      loadFailed = true;
      updateError();
    }
  }
}
export function resetAppearance() {
  motionLoaded = false;
  owner = '';
  videoMuted.set(false);
  generation++;
  localChanges.clear();
  failedChanges.clear();
  loadFailed = false;
  updateError();
  initialChanges = undefined;
  queued = undefined;
  appearance.set({ ...defaults, theme: get(appearance).theme });
}
export function updateAppearance(change: Partial<Appearance>) {
  initialChanges?.push(change);
  if (change.animation_duration_ms !== undefined) motionLoaded = true;
  appearance.update((value) => merge(value, change));
  if (!owner) return;
  const id = ++changeId;
  if (queued?.current === generation) {
    localChanges.delete(queued.id);
    change = merge(queued.change, change);
  }
  queued = { current: generation, id, change };
  localChanges.set(id, change);
  if (saveTimer === undefined) pending = pending.then(save);
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    saveTimer = undefined;
    pending = pending.then(save);
  }, savePause);
}
async function save() {
  const job = queued;
  queued = undefined;
  if (!job || job.current !== generation) return;
  const { current, id, change } = job;
  try {
    await api('/me/appearance', 'PATCH', change);
    if (current === generation) {
      acceptedRevision++;
      acknowledgeSavedChange(id, change);
      updateError();
    }
  } catch {
    if (current === generation) {
      failedChanges.add(id);
      updateError();
    }
  } finally {
    if (!failedChanges.has(id)) localChanges.delete(id);
  }
}
