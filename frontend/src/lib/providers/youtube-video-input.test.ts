import { describe, expect, it } from 'vitest';
import { youtubeVideoIdFromInput } from './youtube-video-input';

describe('YouTube video links', () => {
  it.each([
    'https://www.youtube.com/watch?v=xwdamnfS6IM',
    'https://youtu.be/xwdamnfS6IM?si=shared',
    'https://m.youtube.com/watch?v=xwdamnfS6IM&t=30s',
    'https://music.youtube.com/watch?v=xwdamnfS6IM',
    'https://www.youtube.com/shorts/xwdamnfS6IM',
    'https://www.youtube.com/live/xwdamnfS6IM',
    'https://www.youtube.com/embed/xwdamnfS6IM',
    'https://www.youtube-nocookie.com/embed/xwdamnfS6IM',
  ])('recognizes %s', (url) => {
    expect(youtubeVideoIdFromInput(url)).toBe('xwdamnfS6IM');
  });

  it.each([
    'https://www.youtube.com/@channel',
    'https://www.youtube.com/playlist?list=playlist',
    'https://www.youtube.com/watch?v=invalid',
    'https://youtube.com.example.org/watch?v=xwdamnfS6IM',
    'https://www.youtube.com@evil.example/watch?v=xwdamnfS6IM',
    'javascript:alert(1)',
  ])('leaves non-video or untrusted links alone: %s', (url) => {
    expect(youtubeVideoIdFromInput(url)).toBeNull();
  });
});
