import { describe, expect, it } from 'vitest';
import { playbackProgress } from './youtube-progress';

describe('playback progress events', () => {
  it('move a playing YouTube video by its report', () => {
    expect(
      playbackProgress({
        id: 'session',
        media_id: 'youtube:xwdamnfS6IM',
        position: 150,
        duration: 600,
        watched: false,
      }),
    ).toEqual({
      sessionId: 'session',
      videoId: 'xwdamnfS6IM',
      positionSeconds: 150,
      durationSeconds: 600,
      watchedPercentage: 25,
      isWatched: false,
    });
  });

  it.each([
    // Library media and live streams have no YouTube progress to show.
    { media_id: 'movie-id', position: 10, duration: 600 },
    { media_id: 'youtube:xwdamnfS6IM', position: 10, duration: 0 },
    // Reports from servers before progress was included.
    { media_id: 'youtube:xwdamnfS6IM' },
    {},
  ])('ignore %o', (payload) => {
    expect(playbackProgress(payload)).toBeNull();
  });
});
