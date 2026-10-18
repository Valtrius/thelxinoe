// Desktop playback runs in MPV: each video in its own window, music in one queue.
// Pages read which media is starting or playing from here.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import { api, ApiError } from './api';
import {
  playbackRequest,
  type MediaChoice,
  type Preparation,
} from './playback';
import { toolsApi } from './providers/tools-api';
import { toolOperationActive } from './providers/tools-presentation';

export type NativeView = {
  run: string;
  file_id: string;
  generation: string;
  media_id: string;
  mode: string;
  title: string;
  position: number;
  duration: number;
  paused: boolean;
  status: string;
  music: boolean;
  index: number;
  count: number;
  error: string | null;
  video_ready: boolean;
};
export const ended = (status: string) =>
  ['stopped', 'ended', 'error', 'failed', ''].includes(status);

/** Runs that have not ended, by run. */
export const nativeRuns = new SvelteMap<string, NativeView>();
/** Videos from the click until MPV shows them, by media. */
export const launchingMedia = new SvelteSet<string>();
const reported = new Set<string>();
const watchers = new Map<string, (view: NativeView) => void>();
const ignore = (view: NativeView) => void view;
let failed: (view: NativeView) => void = ignore;

function accept(view: NativeView) {
  reported.add(view.run);
  if (ended(view.status)) nativeRuns.delete(view.run);
  else nativeRuns.set(view.run, view);
  const watcher = watchers.get(view.run);
  if (watcher) watcher(view);
  // The music player shows its own errors.
  else if (view.status === 'failed' && !view.music) failed(view);
}
/** Follows every MPV run; `failure` hears about videos that fail after starting. */
export async function connectNativePlayback(
  failure: (view: NativeView) => void,
): Promise<() => void> {
  failed = failure;
  const unlisten = await listen<NativeView>('mpv-state', (event) =>
    accept(event.payload),
  );
  // A run's own reports are newer than this snapshot.
  for (const view of await invoke<NativeView[]>('mpv_sessions'))
    if (!reported.has(view.run)) accept(view);
  return () => {
    unlisten();
    failed = ignore;
    nativeRuns.clear();
    reported.clear();
  };
}

/** Waits while the managed MPV is still being installed. */
export function waitForTools(preparation: Preparation) {
  return playbackRequest(async () => {
    const snapshot = await toolsApi.get();
    const mpv = snapshot.tools.find((tool) => tool.id === 'mpv');
    if (
      mpv?.preference.source === 'managed' &&
      (!mpv.selectedPath || mpv.diagnostic?.detected === false) &&
      snapshot.operation.tool === 'mpv' &&
      toolOperationActive(snapshot.operation)
    )
      throw new ApiError(503, 'tools_preparing', 'Waiting for playback tools…');
  }, preparation);
}

/** Opens a video in its own MPV window and resolves once MPV shows it. */
export async function launchVideo(
  choice: MediaChoice,
  resolveYoutube: boolean,
  waiting: (value: boolean) => void,
) {
  const run = crypto.randomUUID();
  launchingMedia.add(choice.id);
  const preparation: Preparation = {
    signal: new AbortController().signal,
    waiting,
  };
  try {
    await waitForTools(preparation);
    if (resolveYoutube)
      await playbackRequest(
        () =>
          api('/online/youtube/resolve', 'POST', {
            video_id: choice.id.slice('youtube:'.length),
          }),
        preparation,
      );
    const shown = new Promise<NativeView>((resolve) =>
      watchers.set(run, (view) => {
        if (view.video_ready || ended(view.status)) resolve(view);
      }),
    );
    const started = await invoke<NativeView>('mpv_play', {
      choice,
      queue: null,
      music: false,
      run,
    });
    if (!reported.has(run)) accept(started);
    const view = await shown;
    if (view.status === 'failed')
      throw new Error(view.error ?? 'MPV could not play this media');
  } finally {
    watchers.delete(run);
    launchingMedia.delete(choice.id);
  }
}
