import type { YoutubeCardShortcut, YoutubeVideo } from './types';

export interface YoutubeVideoActions {
  onPlay: (video: YoutubeVideo) => void;
  onBeginning: (video: YoutubeVideo) => void;
  onSignedIn: (video: YoutubeVideo) => void;
  onDownload: (video: YoutubeVideo) => void;
  onDownloadSignedIn: (video: YoutubeVideo) => void;
  onCancelDownload: (video: YoutubeVideo) => void;
  onPinDownload: (video: YoutubeVideo) => void;
  onUnpinDownload: (video: YoutubeVideo) => void;
  onDeleteDownload: (video: YoutubeVideo) => void;
  onMarkWatched: (video: YoutubeVideo) => void;
  onMarkUnwatched: (video: YoutubeVideo) => void;
  onCopy: (video: YoutubeVideo) => void;
  onOpen: (video: YoutubeVideo) => void;
  onAddToWatchlist: (video: YoutubeVideo, watchlistId: number) => void;
  onRemoveFromWatchlist: (video: YoutubeVideo, watchlistId: number) => void;
  onYoutubeCardShortcutsChanged: (shortcuts: YoutubeCardShortcut[]) => void;
}
