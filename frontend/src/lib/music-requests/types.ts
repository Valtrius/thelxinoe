import type { RequestAttention } from '../attention';

export type RequestState =
  | 'pending'
  | 'approved'
  | 'denied'
  | 'cancelled'
  | 'adding'
  | 'searching'
  | 'requested'
  | 'available'
  | 'failed'
  | 'uncertain';

export type Album = {
  /** The album's MusicBrainz release group. */
  external_id: string;
  title: string;
  artist?: string | null;
  year?: number | null;
  /** A same-origin path; the server relays cover art. */
  cover?: string | null;
  in_library?: boolean;
  request?: { id: string; state: RequestState } | null;
};

export type MusicService = {
  id: string;
  name: string;
  kind: string;
  ready: boolean;
};

export type MusicRequest = {
  id: string;
  title: string;
  state: RequestState;
  created_at: number;
  error?: string | null;
  username: string;
  user_id: string;
  attention?: RequestAttention | null;
};

// A declined, cancelled or failed request leaves the album requestable again.
export const requestable = (state?: RequestState) =>
  !state || ['denied', 'cancelled', 'failed'].includes(state);

export const requestLabel = (state: RequestState) =>
  ({
    pending: 'Pending approval',
    approved: 'Requested',
    adding: 'Requested',
    searching: 'Requested',
    requested: 'Requested',
    available: 'Available',
    failed: 'Failed',
    uncertain: 'Needs attention',
    denied: 'Declined',
    cancelled: 'Cancelled',
  })[state];

export const cover = (path?: string | null) =>
  path && path.startsWith('/') && !path.startsWith('//') ? path : '';
