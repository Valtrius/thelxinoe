import { MediaQuery } from 'svelte/reactivity';

// The `touch` and `compact` variants in app.css use these same queries;
// responsive.test.ts fails when the two definitions drift apart.
export const touchMediaQuery = '(pointer: coarse) and (hover: none)';
export const compactMediaQuery = `(max-width: 720px), (max-height: 720px) and ${touchMediaQuery}`;
export const compactViewport = new MediaQuery(compactMediaQuery);
export const touchDevice = new MediaQuery(touchMediaQuery);
