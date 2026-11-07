import type { Page } from '@playwright/test';

// Paused, so that stepping through an animation does not depend on the runner.
export const pauseAnimations = (page: Page) =>
  page.evaluate(() => {
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (keyframes, options) {
      const animation = animate.call(this, keyframes, options);
      animation.pause();
      return animation;
    };
  });

// Moves every paused animation to this share of its duration.
export const seekAnimations = (page: Page, progress: number) =>
  page.evaluate((progress) => {
    for (const animation of document.getAnimations())
      if (animation.playState === 'paused')
        animation.currentTime =
          Number(animation.effect!.getTiming().duration) * progress;
  }, progress);

export const finishAnimations = (page: Page) =>
  page.evaluate(() => {
    for (const animation of document.getAnimations())
      if (animation.playState === 'paused') animation.finish();
  });
