import { captureSession } from './session';
import type { Card } from './media-state';
export type PlaylistDraft = {
  baseRevision: number;
  name: string;
  description: string;
  tracks: Card[];
};
let ownsSession = captureSession();
let drafts = new Map<string, PlaylistDraft>();
export function sessionPlaylistDrafts() {
  if (!ownsSession()) {
    drafts = new Map();
    ownsSession = captureSession();
  }
  return drafts;
}
