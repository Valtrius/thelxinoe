import { describe, expect, it } from 'vitest';
import css from '../app.css?raw';
import { compactMediaQuery, touchMediaQuery } from './responsive';

// Reads `@custom-variant name (@media query);` and the block form
// `@custom-variant name { @media query { @slot; } }`.
function variantQuery(name: string) {
  const declaration = `@custom-variant ${name} `;
  const start = css.indexOf(declaration);
  if (start < 0) throw new Error(`Missing ${name} variant`);
  const rest = css.slice(start + declaration.length);
  const media = rest.indexOf('@media') + '@media'.length;
  const query = rest.startsWith('(@media')
    ? rest.slice(media, rest.indexOf(');'))
    : rest.slice(media, rest.indexOf('{', media));
  return query.replace(/\s+/g, ' ').trim();
}

describe('responsive breakpoints', () => {
  it('match the CSS variants used by the compact and touch layouts', () => {
    expect(variantQuery('compact')).toBe(compactMediaQuery);
    expect(variantQuery('touch')).toBe(touchMediaQuery);
  });
});
