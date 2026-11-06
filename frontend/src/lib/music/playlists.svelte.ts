import { fromStore } from 'svelte/store';
import { api, ApiError, type ServerEvent } from '../api';
import { appearance, updateAppearance } from '../appearance';
import { showToast } from '../providers/toasts';
import { KeyedAsyncQueue } from '../providers/youtube-watchlists';
import { compactViewport } from '../responsive';
import { onServerEvent } from '../server-events.svelte';
import { captureSession } from '../session';
import type { FollowedPlaylist } from '../playback';
import type { MusicItem } from './library';
import {
  sortedPlaylistTracks,
  type PlaylistSort,
  type PlaylistTrack,
  type SortDirection,
} from './playlist-order';

export type PlaylistAccess = 'private' | 'read' | 'add' | 'edit';
export type Playlist = {
  id: string;
  name: string;
  owner_id: string;
  owner: string;
  revision: number;
  count: number;
  access: PlaylistAccess;
  sort_mode: PlaylistSort;
  sort_direction: SortDirection;
  is_owner: boolean;
  can_add: boolean;
  can_edit: boolean;
  /** Whether the track a list was asked about is in it. */
  contains?: boolean;
};
export type PlaylistDetail = Playlist & { items: PlaylistTrack[] };
type PlaylistChange = Partial<
  Pick<Playlist, 'name' | 'access' | 'sort_mode' | 'sort_direction'>
>;

const selectedKey = 'music-selected-playlist-id';
const openKey = 'music-playlist-sidebar-open';
const preferences = fromStore(appearance);
const message = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

/** The playlists of the Music section, and the one its sidebar shows. */
class PlaylistStore {
  lists = $state.raw<Playlist[]>([]);
  /** The manual order of the shown playlist. */
  tracks = $state.raw<PlaylistTrack[]>([]);
  selectedId = $state<string | null>(null);
  /** The shown playlist's tracks have arrived. */
  ready = $state(false);
  mobileOpen = $state(false);

  selected = $derived(
    this.lists.find((list) => list.id === this.selectedId) ?? this.lists[0],
  );
  shown = $derived(
    this.selected
      ? sortedPlaylistTracks(
          this.tracks,
          this.selected.sort_mode,
          this.selected.sort_direction,
        )
      : [],
  );
  canReorder = $derived(
    this.selected?.sort_mode === 'manual' && !!this.selected.can_edit,
  );

  /** The playlist and revision `tracks` shows. */
  loaded: FollowedPlaylist | null = null;
  private user: string | null = null;
  private owner = captureSession();
  private edits = new KeyedAsyncQueue();
  private pendingEdits = 0;
  private loading = false;
  private reloadAfter = false;
  private forceTracks = false;
  private trackRequest = 0;
  private catalogTimer: ReturnType<typeof setTimeout> | undefined;

  get open() {
    return compactViewport.current
      ? this.mobileOpen
      : preferences.current.provider_preferences[openKey] === 'true';
  }

  setOpen(open: boolean) {
    if (compactViewport.current) this.mobileOpen = open;
    else
      updateAppearance({
        provider_preferences: { [openKey]: String(open) },
      });
  }

  /** Follows the server while the Music section is shown. */
  connect(userId: string) {
    if (this.user !== userId || !this.owner()) this.reset();
    this.user = userId;
    this.owner = captureSession();
    this.selectedId ??=
      preferences.current.provider_preferences[selectedKey] ?? null;
    void this.load();
    const disconnect = onServerEvent((event: ServerEvent) => {
      if (event.kind === 'playlists.changed') void this.load();
      else if (event.kind === 'server.reconnected') void this.load(true);
      // Availability and artwork come from the catalog, and scans change a lot.
      else if (event.kind === 'catalog.changed') {
        clearTimeout(this.catalogTimer);
        this.catalogTimer = setTimeout(() => void this.load(true), 2000);
      }
    });
    return () => {
      disconnect();
      clearTimeout(this.catalogTimer);
      this.mobileOpen = false;
    };
  }

  private reset() {
    this.lists = [];
    this.tracks = [];
    this.selectedId = null;
    this.ready = false;
    this.loaded = null;
    this.mobileOpen = false;
  }

  /** Reads what changed. Waits for this tab's own edits, which it would undo. */
  async load(forceTracks = false) {
    this.forceTracks ||= forceTracks;
    if (this.loading || this.pendingEdits) {
      this.reloadAfter = true;
      return;
    }
    this.loading = true;
    const owned = captureSession();
    try {
      const { items } = await api<{ items: Playlist[] }>('/playlists');
      if (!owned() || !this.owner()) return;
      if (this.pendingEdits) {
        this.reloadAfter = true;
        return;
      }
      this.lists = items;
      // A list chosen by default stays chosen when others appear before it.
      this.selectedId ??= this.selected?.id ?? null;
      await this.loadTracks();
    } catch {
      // The next change or reconnection reads again.
    } finally {
      this.loading = false;
      if (this.reloadAfter && !this.pendingEdits) {
        this.reloadAfter = false;
        void this.load();
      }
    }
  }

  private async loadTracks() {
    const list = this.selected;
    if (!list) {
      this.tracks = [];
      this.loaded = null;
      this.ready = true;
      return;
    }
    if (
      !this.forceTracks &&
      this.loaded?.id === list.id &&
      this.loaded.revision === list.revision
    )
      return;
    this.forceTracks = false;
    if (this.loaded && this.loaded.id !== list.id) {
      this.tracks = [];
      this.ready = false;
    }
    const request = ++this.trackRequest;
    const owned = captureSession();
    try {
      const detail = await api<PlaylistDetail>(`/playlists/${list.id}`);
      if (
        !owned() ||
        !this.owner() ||
        request !== this.trackRequest ||
        this.selected?.id !== list.id
      )
        return;
      if (this.pendingEdits) {
        this.reloadAfter = true;
        return;
      }
      this.tracks = detail.items;
      this.loaded = { id: list.id, revision: detail.revision };
      this.ready = true;
    } catch (error) {
      if (!(error instanceof ApiError) || error.status !== 404) throw error;
      // Made private or deleted since the lists were read; its event follows.
      this.tracks = [];
      this.loaded = { id: list.id, revision: list.revision };
      this.ready = true;
    }
  }

  select(id: string) {
    if (id === this.selected?.id) return;
    this.selectedId = id;
    this.tracks = [];
    this.loaded = null;
    this.ready = false;
    updateAppearance({ provider_preferences: { [selectedKey]: id } });
    void this.load();
  }

  private patchList(id: string, change: Partial<Playlist>) {
    this.lists = this.lists.map((list) =>
      list.id === id ? { ...list, ...change } : list,
    );
  }

  /**
   * Records an edit's revision. One past what the edit itself made means someone
   * else edited too.
   */
  private settle(id: string, revision: number, wrote: boolean) {
    const known = this.lists.find((list) => list.id === id)?.revision;
    if (known !== undefined && revision > known)
      this.patchList(id, { revision });
    if (this.loaded?.id !== id) return;
    if (revision > this.loaded.revision + (wrote ? 1 : 0))
      this.forceTracks = true;
    this.loaded = { id, revision };
  }

  /** Runs an edit after those before it on the playlist. */
  private async edit<T extends { revision: number; added?: number }>(
    id: string,
    request: () => Promise<T>,
  ): Promise<T> {
    this.pendingEdits++;
    const owned = captureSession();
    try {
      const result = await this.edits.run(id, request);
      if (owned()) this.settle(id, result.revision, result.added !== 0);
      return result;
    } catch (error) {
      if (owned()) this.forceTracks = true;
      throw error;
    } finally {
      this.pendingEdits--;
      if (owned() && !this.pendingEdits) {
        this.reloadAfter = false;
        void this.load();
      }
    }
  }

  private failed(error: unknown) {
    showToast({
      key: 'playlist-change',
      tone: 'error',
      title: 'Playlist change failed',
      message: message(error),
    });
  }

  /** Adds tracks a list doesn't have yet. Resolves to how many it took. */
  async addTracks(id: string, tracks: MusicItem[]) {
    const ids = [...new Set(tracks.map((track) => track.id))];
    const showing = this.selected?.id === id;
    if (showing && this.ready) {
      const present = new Set(this.tracks.map((track) => track.id));
      const addedAt = Math.floor(Date.now() / 1000);
      this.tracks = [
        ...this.tracks,
        ...tracks
          .filter((track) => !present.has(track.id))
          .map((track) => ({ ...track, added_at: addedAt })),
      ];
    }
    const { added } = await this.edit(id, () =>
      api<{ added: number; revision: number }>(
        `/playlists/${id}/items`,
        'POST',
        { items: ids },
      ),
    );
    const count = this.lists.find((list) => list.id === id)?.count;
    if (count !== undefined) this.patchList(id, { count: count + added });
    return added;
  }

  async removeTrack(id: string, media: string) {
    this.tracks = this.tracks.filter((track) => track.id !== media);
    const count = this.lists.find((list) => list.id === id)?.count;
    if (count) this.patchList(id, { count: count - 1 });
    try {
      await this.edit(id, () =>
        api<{ revision: number }>(
          `/playlists/${id}/items/${encodeURIComponent(media)}`,
          'DELETE',
        ),
      );
    } catch (error) {
      this.failed(error);
    }
  }

  /** Puts a dragged track in its new place within `order`. */
  async moveTrack(id: string, order: string[], moved: string) {
    const before = order[order.indexOf(moved) + 1] ?? null;
    if (this.tracks.map((track) => track.id).join() === order.join()) return;
    const byId = new Map(this.tracks.map((track) => [track.id, track]));
    this.tracks = order.flatMap((key) => byId.get(key) ?? []);
    try {
      await this.edit(id, () =>
        api<{ revision: number }>(
          `/playlists/${id}/items/${encodeURIComponent(moved)}/move`,
          'POST',
          { before },
        ),
      );
    } catch (error) {
      this.failed(error);
    }
  }

  /** Changes a playlist; a new name shows once it was accepted. */
  async update(id: string, change: PlaylistChange): Promise<string | void> {
    const { name, ...immediate } = change;
    this.patchList(id, immediate);
    try {
      await this.edit(id, () =>
        api<{ revision: number }>(`/playlists/${id}`, 'PATCH', change),
      );
      if (name !== undefined) this.patchList(id, { name });
    } catch (error) {
      if (name !== undefined) return message(error);
      this.failed(error);
    }
  }

  /** Creates a playlist and shows it, or resolves to why it can't exist. */
  async create(name: string): Promise<string | void> {
    try {
      const { id } = await api<{ id: string }>('/playlists', 'POST', { name });
      this.select(id);
      await this.load();
    } catch (error) {
      return message(error);
    }
  }

  async remove(id: string) {
    const lists = this.lists;
    this.lists = lists.filter((list) => list.id !== id);
    if (this.loaded?.id === id || this.selectedId === id) {
      this.selectedId = null;
      this.tracks = [];
      this.loaded = null;
      this.ready = false;
    }
    try {
      await api(`/playlists/${id}`, 'DELETE');
    } catch (error) {
      this.lists = lists;
      this.failed(error);
    }
    await this.load(true);
  }
}

export const playlists = new PlaylistStore();
