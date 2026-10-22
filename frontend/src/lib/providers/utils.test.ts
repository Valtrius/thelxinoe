import { describe, expect, it } from 'vitest';
import { twitchPreview, twitchPreviewSet, youtubeAvatarSet } from './utils';

describe('image sizes', () => {
  it('sizes Twitch previews from the API template or a stored size', () => {
    const stored =
      'https://static-cdn.jtvnw.net/previews-ttv/live_user_someone-640x360.jpg';
    expect(twitchPreview(stored, 320)).toBe(
      'https://static-cdn.jtvnw.net/previews-ttv/live_user_someone-320x180.jpg',
    );
    expect(
      twitchPreview(
        'https://static-cdn.jtvnw.net/previews-ttv/live_user_someone-{width}x{height}.jpg',
        640,
      ),
    ).toBe(stored);
    expect(twitchPreviewSet(stored)).toBe(
      [
        'https://static-cdn.jtvnw.net/previews-ttv/live_user_someone-320x180.jpg 320w',
        'https://static-cdn.jtvnw.net/previews-ttv/live_user_someone-480x270.jpg 480w',
        `${stored} 640w`,
      ].join(),
    );
    expect(
      twitchPreviewSet('https://static-cdn.jtvnw.net/previews-ttv/other.jpg'),
    ).toBeUndefined();
  });

  it('sizes YouTube channel avatars up to the size the API gives', () => {
    expect(
      youtubeAvatarSet(
        'https://yt3.ggpht.com/ytc/channel=s240-c-k-c0x00ffffff-no-rj',
      ),
    ).toBe(
      [36, 72, 108, 176, 240]
        .map(
          (size) =>
            `https://yt3.ggpht.com/ytc/channel=s${size}-c-k-c0x00ffffff-no-rj ${size}w`,
        )
        .join(),
    );
    expect(
      youtubeAvatarSet('https://yt3.googleusercontent.com/channel=s88'),
    ).toContain('https://yt3.googleusercontent.com/channel=s72 72w');
    expect(
      youtubeAvatarSet('https://example.com/channel=s240'),
    ).toBeUndefined();
    expect(youtubeAvatarSet('https://yt3.ggpht.com/channel')).toBeUndefined();
  });
});
