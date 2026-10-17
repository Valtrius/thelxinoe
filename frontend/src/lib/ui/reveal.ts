import { cubicOut } from 'svelte/easing';
import { slide, type SlideParams } from 'svelte/transition';
import { getAnimationDuration } from '../motion';

// Slide a block open or closed on the user's animation clock. Use as
// `transition:reveal` on content toggled by an {#if}; a 0 ms preference or
// reduced motion makes it instant.
// The element must contain its children's vertical margins (padding, border,
// grid or flow-root): slide measures the box at rest, then clips overflow while
// animating, and margins that collapse through the box make the end jump.
export function reveal(node: HTMLElement, params: SlideParams = {}) {
  return slide(node, {
    duration: getAnimationDuration(),
    easing: cubicOut,
    ...params,
  });
}
