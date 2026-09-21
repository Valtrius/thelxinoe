import { cubicOut } from 'svelte/easing';
import type { TransitionConfig } from 'svelte/transition';
import { prefersReducedMotion } from './layout-animation';

export function playerReveal(node: HTMLElement): TransitionConfig {
  const height = node.getBoundingClientRect().height;
  const margin = Number.parseFloat(getComputedStyle(node).marginBottom) || 0;
  return {
    duration: prefersReducedMotion() ? 0 : 240,
    easing: cubicOut,
    // Keep the video at its full size while revealing it from above. The
    // collapsing margin lets the following page content move with its edge.
    css: (t, u) => `
      transform: translateY(${-u * height}px);
      clip-path: inset(${u * 100}% 0 0 0);
      margin-bottom: ${t * margin - u * height}px;
    `,
  };
}
