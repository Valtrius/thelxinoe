import { inBackground, trackPageWork } from './page-readiness';

export type PageSource = 'catalog' | 'discover' | 'youtube' | 'twitch' | 'kick';
type Entry = { source: PageSource; json: string; at: number; stale?: boolean };
type Pending = { source: PageSource; value: Promise<unknown> };

export class NavigationCache {
  private entries = new Map<string, Entry>();
  private pending = new Map<string, Pending>();
  private bytes = 0;
  private session = 0;

  constructor(
    private maxBytes = 8 * 1024 * 1024,
    private maxEntries = 32,
  ) {}

  clear() {
    this.session++;
    this.entries.clear();
    this.pending.clear();
    this.bytes = 0;
  }

  invalidate(...sources: PageSource[]) {
    for (const [key, entry] of this.entries)
      if (sources.includes(entry.source)) this.remove(key);
    for (const [key, entry] of this.pending)
      if (sources.includes(entry.source)) this.pending.delete(key);
  }

  // Keeps the snapshots, so pages still show them at once, but rereads them
  // behind the page on its next read.
  expire(...sources: PageSource[]) {
    for (const entry of this.entries.values())
      if (sources.includes(entry.source)) entry.stale = true;
  }

  private remove(key: string) {
    const previous = this.entries.get(key);
    if (previous) this.bytes -= (key.length + previous.json.length) * 2;
    this.entries.delete(key);
  }

  read<T>(
    source: PageSource,
    key: string,
    load: () => Promise<T>,
    accept: (value: T) => void | Promise<void>,
    freshFor = 30_000,
  ): Promise<void> {
    const owner = this.session;
    const cacheKey = `${source}:${key}`;
    let cached = this.entries.get(cacheKey);
    if (cached && Date.now() - cached.at > 5 * 60_000) {
      this.remove(cacheKey);
      cached = undefined;
    }
    if (cached) {
      this.entries.delete(cacheKey);
      this.entries.set(cacheKey, cached);
    }
    const restored = cached ? accept(JSON.parse(cached.json) as T) : undefined;
    if (cached && !cached.stale && Date.now() - cached.at < freshFor)
      return Promise.resolve(restored);
    const read = () =>
      trackPageWork(() => {
        let pending = this.pending.get(cacheKey);
        if (!pending) {
          const entry: Pending = { source, value: load() };
          this.pending.set(cacheKey, entry);
          while (this.pending.size > this.maxEntries)
            this.pending.delete(this.pending.keys().next().value!);
          entry.value = entry.value
            .then(
              (value) => {
                if (
                  this.session === owner &&
                  this.pending.get(cacheKey) === entry
                ) {
                  // Serialize before the view can wrap or mutate the response. This
                  // also bounds retained payload memory and isolates each reader.
                  const json = JSON.stringify(value);
                  this.remove(cacheKey);
                  if ((cacheKey.length + json.length) * 2 <= this.maxBytes) {
                    this.entries.set(cacheKey, {
                      source,
                      json,
                      at: Date.now(),
                    });
                    this.bytes += (cacheKey.length + json.length) * 2;
                    while (
                      this.bytes > this.maxBytes ||
                      this.entries.size > this.maxEntries
                    )
                      this.remove(this.entries.keys().next().value!);
                  }
                }
                return value;
              },
              (error: unknown) => {
                if (this.pending.get(cacheKey) === entry) this.remove(cacheKey);
                throw error;
              },
            )
            .finally(() => {
              if (this.pending.get(cacheKey) === entry)
                this.pending.delete(cacheKey);
            });
          pending = entry;
        }
        return pending.value;
      });
    const result = cached ? inBackground(read) : read();
    void result.catch(() => {});
    return Promise.resolve(restored).then(async () => {
      const value = await result;
      if (owner === this.session) await accept(structuredClone(value) as T);
    });
  }
}

export const navigationCache = new NavigationCache();

// `progress` marks a playback report that repeats its session's state: it only
// moves a position, which pages may show a moment late.
export function invalidatePageEvent(kind: string, progress = false) {
  if (progress) {
    navigationCache.expire('catalog', 'youtube');
    return;
  }
  if (kind === 'server.reconnected' || kind === 'account.profile.changed') {
    navigationCache.invalidate(
      'catalog',
      'discover',
      'youtube',
      'twitch',
      'kick',
    );
    return;
  }
  if (
    [
      'catalog.changed',
      'catalog.status.changed',
      'media-state.changed',
      'playlists.changed',
      'playback.changed',
    ].includes(kind)
  )
    navigationCache.invalidate('catalog');
  if (
    [
      'youtube.changed',
      'online.download.changed',
      'playback.changed',
      'media-state.changed',
      'playlists.changed',
    ].includes(kind)
  )
    navigationCache.invalidate('youtube');
  if (['online.account.changed', 'online.configuration.changed'].includes(kind))
    navigationCache.invalidate('youtube', 'twitch', 'kick');
  if (
    [
      'stack.changed',
      'catalog.status.changed',
      'requests.changed',
      'attention.changed',
    ].includes(kind)
  )
    navigationCache.invalidate('discover');
}

export function invalidatePageWrite(path: string) {
  if (/^\/(catalog|playlists)(\/|\?|$)/.test(path))
    navigationCache.invalidate('catalog');
  if (path.startsWith('/online/youtube') && path !== '/online/youtube/browse')
    navigationCache.invalidate('youtube');
  if (path.startsWith('/online/twitch')) navigationCache.invalidate('twitch');
  if (path.startsWith('/online/kick')) navigationCache.invalidate('kick');
  if (path.startsWith('/seerr/'))
    navigationCache.invalidate('discover', 'catalog');
  if (path.startsWith('/admin/'))
    navigationCache.invalidate(
      'catalog',
      'discover',
      'youtube',
      'twitch',
      'kick',
    );
  // A player reports its position every few seconds; its state changes arrive
  // as events.
  if (/^\/playback\/[^/]+\/progress$/.test(path))
    navigationCache.expire('catalog', 'youtube');
  else if (/^\/playback(?:\/[^/]+)?$/.test(path))
    navigationCache.invalidate('catalog', 'youtube');
}
