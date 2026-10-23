import { describe, expect, it } from 'vitest';
import { libraryArtworkSet } from './artwork';

describe('library artwork sizes', () => {
  it('keeps origins and grants for library images and manager posters', () => {
    for (const path of [
      '/api/v1/catalog/item/artwork',
      '/api/v1/catalog/titles/radarr-1/12/artwork/poster',
    ]) {
      for (const origin of ['', 'https://media.example']) {
        expect(libraryArtworkSet(`${origin}${path}?grant=stable`)).toBe(
          [500, 1000]
            .map(
              (width) =>
                `${origin}${path}?grant=stable&width=${width} ${width}w`,
            )
            .join(', '),
        );
      }
    }
  });
  it('leaves backgrounds, provider images and mock artwork alone', () => {
    for (const source of [
      '',
      '/api/v1/catalog/titles/radarr-1/12/artwork/fanart',
      '/api/v1/online/youtube/videos/abcdefghijk/artwork',
      '/__mock/art/poster',
      'https://image.tmdb.org/t/p/w500/poster.jpg',
    ]) {
      expect(libraryArtworkSet(source)).toBeUndefined();
    }
  });
});
