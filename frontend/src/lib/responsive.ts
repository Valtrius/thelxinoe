import { MediaQuery } from 'svelte/reactivity';

export const compactMediaQuery =
  '(max-width: 720px), (max-height: 720px) and (pointer: coarse) and (hover: none)';
export const compactViewport = new MediaQuery(compactMediaQuery);
export const touchDevice = new MediaQuery(
  '(pointer: coarse) and (hover: none)',
);
