import { describe, expect, it } from 'vitest';
import { modulePath, qrCode } from './qr';

describe('QR rendering', () => {
  it('merges adjacent dark modules in a row', () => {
    expect(
      modulePath([
        [true, true, false, true],
        [false, false, false, false],
        [false, true, true, true],
      ]),
    ).toBe('M0 0h2v1h-2zM3 0h1v1h-1zM1 2h3v1h-3z');
  });

  it('keeps a light quiet zone around an authenticator URI', () => {
    const { size, path } = qrCode(
      'otpauth://totp/Thelxinoe:alice%40media.example.com?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Thelxinoe',
    );
    expect(size).toBeGreaterThanOrEqual(21 + 8);
    const starts = [...path.matchAll(/M(\d+) (\d+)h(\d+)/g)].map((m) =>
      m.slice(1).map(Number),
    );
    expect(starts.length).toBeGreaterThan(0);
    for (const [x, y, width] of starts) {
      expect(x).toBeGreaterThanOrEqual(4);
      expect(y).toBeGreaterThanOrEqual(4);
      expect(x + width).toBeLessThanOrEqual(size - 4);
      expect(y).toBeLessThan(size - 4);
    }
  });
});
