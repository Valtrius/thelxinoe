import { cubicOut } from 'svelte/easing';
import type { TransitionConfig } from 'svelte/transition';
import { getAnimationDuration } from './motion';

export function playerReveal(node: HTMLElement): TransitionConfig {
  const height = node.getBoundingClientRect().height;
  const margin = Number.parseFloat(getComputedStyle(node).marginBottom) || 0;
  return {
    duration: getAnimationDuration(),
    easing: cubicOut,
    // Keep the video at its full size while revealing it from above. The
    // collapsing margin lets the following page content move with its edge;
    // a pinned player's frame shrinks with it so the page padding follows.
    css: (t, u) => `
      transform: translateY(${-u * height}px);
      clip-path: inset(${u * 100}% 0 0 0);
      margin-bottom: ${t * margin - u * height}px;
    `,
  };
}

// Everything else the player adds to the page layout follows the reveal, so
// the page content ends where it rests without the player instead of jumping.

/** The resize handle below the video grows and shrinks with its edge. */
export function playerHandleReveal(node: HTMLElement): TransitionConfig {
  const height = node.getBoundingClientRect().height;
  return {
    duration: getAnimationDuration(),
    easing: cubicOut,
    css: (t) => `height: ${t * height}px; overflow: hidden;`,
  };
}

/**
 * A frame that pulls the player over the page padding returns that padding
 * gradually, while the player itself stays against the top edge.
 */
export function playerFrameReveal(node: HTMLElement): TransitionConfig {
  const margin = Number.parseFloat(getComputedStyle(node).marginTop) || 0;
  if (!margin) return { duration: 0 };
  return {
    duration: getAnimationDuration(),
    easing: cubicOut,
    css: (t, u) =>
      `margin-top: ${t * margin}px; transform: translateY(${u * margin}px);`,
  };
}
