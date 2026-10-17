import { getContext, setContext } from 'svelte';

// Settings search renders whole pages and hides the cards that did not match.
// Cards outside a search, or without a title, are always shown.
export type SettingsCardFilter = (title: string) => boolean;

const key = Symbol('settings card filter');

export function filterSettingsCards(filter: SettingsCardFilter) {
  setContext(key, filter);
}

export function settingsCardFilter() {
  return getContext<SettingsCardFilter | undefined>(key);
}
