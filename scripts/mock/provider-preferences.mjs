import { readFileSync } from 'node:fs';

const keys = readFileSync(
  new URL(
    '../../apps/server/src/provider_preference_keys.txt',
    import.meta.url,
  ),
  'utf8',
)
  .trim()
  .split('\n');

export const providerPreferencesError =
  'Invalid player or provider preferences';

export const acceptsProviderPreferences = (preferences = {}) =>
  Object.keys(preferences).every((key) => keys.includes(key));
