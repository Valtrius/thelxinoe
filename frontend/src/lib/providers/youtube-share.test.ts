import { describe, expect, it } from 'vitest';
import { sharedYoutubeVideoId } from './youtube-share';

describe('Shared YouTube videos', () => {
  it.each<Record<string, string>>([
    { share_text: 'https://youtu.be/xwdamnfS6IM?si=shared' },
    { share_url: 'https://www.youtube.com/watch?v=xwdamnfS6IM' },
    { share_text: 'Watch this: https://youtu.be/xwdamnfS6IM!' },
    { share_title: 'Title', share_text: 'youtu.be/xwdamnfS6IM' },
  ])('finds the video in %o', (share) => {
    expect(sharedYoutubeVideoId(new URLSearchParams(share))).toBe(
      'xwdamnfS6IM',
    );
  });

  it.each<Record<string, string>>([
    { share_text: 'Interesting' },
    { share_text: 'https://www.youtube.com/@channel' },
    { share_url: 'https://example.org/watch?v=xwdamnfS6IM' },
    {},
  ])('finds no video in %o', (share) => {
    expect(sharedYoutubeVideoId(new URLSearchParams(share))).toBeNull();
  });
});
