import { get, writable } from 'svelte/store';
import { api } from '../api';
import { captureSession } from '../session';

export type Provider = 'youtube' | 'twitch' | 'kick';
export type ProviderAvailability = Record<Provider, boolean>;
export const providers = writable<ProviderAvailability>({
  youtube: true,
  twitch: true,
  kick: true,
});
export const providerNames = {
  youtube: 'YouTube',
  twitch: 'Twitch',
  kick: 'Kick',
} as const;
let revision = 0;

export function acceptProviders(value: ProviderAvailability) {
  revision++;
  providers.set(value);
}

export async function loadProviders() {
  const current = ++revision;
  const ownsSession = captureSession();
  const value = await api<ProviderAvailability>('/online/providers');
  if (current === revision && ownsSession()) providers.set(value);
}

export async function saveProvider(provider: Provider, enabled: boolean) {
  const value = await api<ProviderAvailability>(
    `/admin/online/providers/${provider}`,
    'PUT',
    { enabled },
  );
  acceptProviders(value);
}

export function providerEnabled(mediaOrSection: string) {
  const provider = mediaOrSection.split(':')[0].toLowerCase();
  return (
    !Object.hasOwn(providerNames, provider) ||
    get(providers)[provider as Provider]
  );
}
