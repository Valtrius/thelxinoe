import { youtubeVideoIdFromInput } from './youtube-video-input';

// The manifest's share target opens `/` with these parameters.
const shareParameters = ['share_url', 'share_text', 'share_title'];
const pendingShare = 'thelxinoe:shared-youtube-video';

// Shared text surrounds the link with words that can look like bare video IDs,
// so only links count.
export function sharedYoutubeVideoId(query: URLSearchParams): string | null {
  for (const name of shareParameters)
    for (const word of query.get(name)?.split(/\s+/) ?? []) {
      const id =
        word.includes('/') &&
        youtubeVideoIdFromInput(word.replace(/[.,;!?)\]]+$/, ''));
      if (id) return id;
    }
  return null;
}

// Navigation rewrites the address, so a share moves to the tab's session
// storage first. It waits there while the user signs in.
export function captureSharedYoutubeVideo() {
  const url = new URL(location.href);
  if (!shareParameters.some((name) => url.searchParams.has(name))) return;
  sessionStorage.setItem(
    pendingShare,
    sharedYoutubeVideoId(url.searchParams) ?? '',
  );
  for (const name of shareParameters) url.searchParams.delete(name);
  history.replaceState(history.state, '', url);
}

// An empty ID is a share without a YouTube video link.
export function takeSharedYoutubeVideo(): string | null {
  const id = sessionStorage.getItem(pendingShare);
  sessionStorage.removeItem(pendingShare);
  return id;
}
