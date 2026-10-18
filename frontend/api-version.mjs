import { readFileSync } from 'node:fs';

// The web client speaks the API version declared once in crates/core, so a
// server API change cannot ship with a web app that still sends the old one.
/** @param {string} source */
export function parseApiVersion(source) {
  const match = /^pub const API_VERSION: u32 = (\d+);/m.exec(source);
  if (!match)
    throw Error('crates/core/src/lib.rs does not declare API_VERSION');
  return Number(match[1]);
}
export function apiVersion() {
  return parseApiVersion(
    readFileSync(new URL('../crates/core/src/lib.rs', import.meta.url), 'utf8'),
  );
}
