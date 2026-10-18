import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { parseApiVersion } from '../../api-version.mjs';
import { clientApi } from './api';

describe('client API version', () => {
  it('is the API_VERSION declared in crates/core', () => {
    const source = readFileSync(
      new URL('../../../crates/core/src/lib.rs', import.meta.url),
      'utf8',
    );
    expect(clientApi).toBe(parseApiVersion(source));
    expect(Number.isInteger(clientApi) && clientApi > 0).toBe(true);
  });
  it('reads the current version, not the oldest accepted one', () => {
    const source = [
      'pub const API_MIN_VERSION: u32 = 3;',
      'pub const API_VERSION: u32 = 12;',
    ].join('\r\n');
    expect(parseApiVersion(source)).toBe(12);
    expect(() =>
      parseApiVersion('pub const API_MIN_VERSION: u32 = 3;'),
    ).toThrow('does not declare API_VERSION');
  });
});
