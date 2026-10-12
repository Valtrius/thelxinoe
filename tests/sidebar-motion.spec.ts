import { test, expect } from '@playwright/test';
import { installSidebarFixture } from './helpers/sidebar-fixture';

const pages = [
  'Home',
  'Discover',
  'Movies',
  'Shows',
  'Music',
  'Playlists',
  'YouTube',
  'Twitch',
  'Kick',
  'Statistics',
];
const cases: { section: string; width: number; scrollbars?: 'overlay' }[] = [
  ...pages.map((section) => ({ section, width: 1440 })),
  ...['Home', 'Discover', 'YouTube', 'Statistics'].map((section) => ({
    section,
    width: 960,
  })),
  ...['Home', 'YouTube'].map((section) => ({ section, width: 1920 })),
  { section: 'YouTube', width: 1280 },
  { section: 'YouTube', width: 1440, scrollbars: 'overlay' },
];
for (const { section, width, scrollbars } of cases) {
  test(`${section} keeps sidebar motion continuous, grouped and reversible at ${width}px${scrollbars ? ` with ${scrollbars} scrollbars` : ''}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 1000 });
    const fixture = await installSidebarFixture(page, section);
    await page.goto('/');
    if (section === 'YouTube')
      await page.addStyleTag({
        content:
          scrollbars === 'overlay'
            ? '[data-feed-content] { scrollbar-gutter: auto; scrollbar-width: none; }'
            : '[data-feed-content] { scrollbar-gutter: stable; scrollbar-width: auto; } [data-feed-content]::-webkit-scrollbar { display: block; width: 15px; }',
      });
    await expect(page.locator('.page-header h1')).toHaveText(section);
    await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
    await page.evaluate(async () => {
      await document.fonts.ready;
      await new Promise((resolve) => setTimeout(resolve, 250));
      const animate = Element.prototype.animate;
      Element.prototype.animate = function (frames, options) {
        const animation = animate.call(this, frames, options);
        animation.pause();
        animation.currentTime = 0;
        return animation;
      };
    });
    const sample = () =>
      page.evaluate(() => {
        const selectors = [
          '.page-header h1',
          '.media-row h2',
          '.media-row button',
          '.media-row-rail',
          '.media-row-rail > :first-child',
          '.settings-navigation',
          '.settings-panels > .panel',
          '.settings-content select',
          '[data-feed-content]',
          'form:has(input[aria-label="Search YouTube feed"])',
          '[data-card-group-header]',
          '[data-card-group-header] .feed-group-label',
          '[data-card-grid] > :first-child',
          '.statistics > div button',
          '.statistics > div select',
          '.statistics article',
          '.statistics section.panel',
          '[data-sidebar-panel]',
          '[data-youtube-watchlist-scale]',
        ];
        return Object.fromEntries(
          selectors.map((selector) => [
            selector,
            [...document.querySelectorAll(selector)]
              .filter((element) => {
                const rect = element.getBoundingClientRect();
                return (
                  getComputedStyle(element).visibility !== 'hidden' &&
                  rect.width > 0 &&
                  rect.height > 0 &&
                  rect.top < innerHeight &&
                  rect.bottom > 0
                );
              })
              .map((element) => {
                const { x, y, width, height } = element.getBoundingClientRect();
                return { x, y, width, height };
              }),
          ]),
        );
      });
    const seek = (time: number) =>
      page.evaluate((time) => {
        for (const animation of document.getAnimations())
          if (
            animation instanceof Animation &&
            animation.effect instanceof KeyframeEffect &&
            animation.effect
              .getKeyframes()
              .some(
                (frame) =>
                  frame.transform ||
                  frame.width ||
                  frame.clipPath ||
                  frame['--feed-label-scale-x'],
              )
          )
            animation.currentTime = time;
      }, time);
    const finish = () =>
      page.evaluate(async () => {
        for (const animation of document.getAnimations())
          if (
            animation.effect instanceof KeyframeEffect &&
            animation.effect
              .getKeyframes()
              .some(
                (frame) =>
                  frame.transform ||
                  frame.width ||
                  frame.clipPath ||
                  frame['--feed-label-scale-x'],
              )
          )
            animation.finish();
        await new Promise(requestAnimationFrame);
      });
    const toggle = (name: string) =>
      page.evaluate(async (name) => {
        document
          .querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`)!
          .click();
        await new Promise(requestAnimationFrame);
      }, name);
    const evidence: unknown[] = [];
    function continuous(
      before: Awaited<ReturnType<typeof sample>>,
      next: Awaited<ReturnType<typeof sample>>,
    ) {
      for (const [selector, rects] of Object.entries(before))
        rects.forEach((rect, index) => {
          const current = next[selector][index];
          if (!current) return;
          for (const axis of ['x', 'y', 'width', 'height'] as const)
            expect(
              Math.abs(current[axis] - rect[axis]),
              `${selector} ${index} ${axis}`,
            ).toBeLessThan(1.5);
        });
    }
    function youtubeEdges(
      before: Awaited<ReturnType<typeof sample>>,
      next: Awaited<ReturnType<typeof sample>>,
    ) {
      if (section !== 'YouTube') return;
      const search = 'form:has(input[aria-label="Search YouTube feed"])';
      const beforeSearch = before[search][0];
      const nextSearch = next[search][0];
      expect(
        Math.abs(
          nextSearch.x + nextSearch.width - beforeSearch.x - beforeSearch.width,
        ),
        'Search right edge stays anchored',
      ).toBeLessThan(0.5);
      const beforeFeed = before['[data-feed-content]'][0];
      const nextFeed = next['[data-feed-content]'][0];
      const header = before['[data-card-group-header]'][0];
      const gutter = beforeFeed.x + beforeFeed.width - header.x - header.width;
      for (const rect of next['[data-card-group-header]']) {
        expect(
          Math.abs(rect.x - nextFeed.x),
          'Date heading left edge',
        ).toBeLessThan(0.5);
        expect(
          Math.abs(nextFeed.x + nextFeed.width - rect.x - rect.width - gutter),
          'Date heading right edge follows the feed with a fixed scrollbar gutter',
        ).toBeLessThan(0.5);
      }
      before['[data-card-group-header] .feed-group-label'].forEach(
        (rect, index) => {
          const current =
            next['[data-card-group-header] .feed-group-label'][index];
          if (current)
            expect(
              Math.abs(current.width - rect.width),
              'Date label keeps its width',
            ).toBeLessThan(0.5);
        },
      );
    }
    for (const name of ['Collapse sidebar', 'Expand sidebar']) {
      const before = await sample();
      await toggle(name);
      const start = await sample();
      await testInfo.attach(`${name}-start.json`, {
        body: JSON.stringify({ before, start }),
        contentType: 'application/json',
      });
      continuous(before, start);
      youtubeEdges(before, start);
      const frames = [];
      for (const time of [40, 80, 120, 180, 199]) {
        await seek(time);
        const geometry = await sample();
        youtubeEdges(before, geometry);
        frames.push({ time, geometry });
      }
      await page.screenshot({
        path: testInfo.outputPath(`${name.split(' ')[0]}-middle.png`),
      });
      const last = await sample();
      await finish();
      const final = await sample();
      continuous(last, final);
      for (const frame of frames)
        for (const selector of [
          '.media-row-rail > :first-child',
          '.media-row h2',
          '.media-row button',
          '.settings-navigation',
          '.statistics > div button',
          '.statistics > div select',
          '[data-card-group-header] .feed-group-label',
        ]) {
          before[selector].forEach((rect, index) => {
            const current = frame.geometry[selector][index];
            if (current)
              expect(
                Math.abs(current.height - rect.height),
                `${selector} stays fixed height`,
              ).toBeLessThan(0.5);
          });
        }
      evidence.push({ name, before, start, frames, final });
    }
    await toggle('Collapse sidebar');
    await seek(70);
    const interrupted = await sample();
    await toggle('Expand sidebar');
    continuous(interrupted, await sample());
    await finish();
    const scrollTarget =
      section === 'YouTube' ? '[data-feed-content]' : '.workspace-scroll';
    await page.locator(scrollTarget).evaluate((node) => (node.scrollTop = 240));
    await page.evaluate(() => new Promise(requestAnimationFrame));
    const scrolled = await sample();
    const scrollTop = await page
      .locator(scrollTarget)
      .evaluate((node) => node.scrollTop);
    await toggle('Collapse sidebar');
    continuous(scrolled, await sample());
    await seek(70);
    const scrolledMiddle = await sample();
    await toggle('Expand sidebar');
    continuous(scrolledMiddle, await sample());
    await finish();
    expect(
      await page.locator(scrollTarget).evaluate((node) => node.scrollTop),
    ).toBe(scrollTop);
    if (section === 'YouTube') {
      for (const name of ['Close watchlists', 'Open watchlists']) {
        const before = await sample();
        await toggle(name);
        const start = await sample();
        continuous(before, start);
        youtubeEdges(before, start);
        const frames = [];
        for (const time of [40, 80, 120, 180, 199]) {
          await seek(time);
          const geometry = await sample();
          youtubeEdges(before, geometry);
          frames.push({ time, geometry });
        }
        const last = await sample();
        await finish();
        const final = await sample();
        continuous(last, final);
        evidence.push({ name, before, frames, final });
      }
      await toggle('Collapse sidebar');
      await seek(70);
      const beforeWatchlist = await sample();
      await toggle('Close watchlists');
      continuous(beforeWatchlist, await sample());
      await seek(90);
      const beforeSidebar = await sample();
      await toggle('Expand sidebar');
      const handoff = await sample();
      expect(handoff['[data-youtube-watchlist-scale]']).toHaveLength(
        beforeSidebar['[data-youtube-watchlist-scale]'].length,
      );
      continuous(beforeSidebar, handoff);
      await finish();
      await toggle('Open watchlists');
      await finish();
      await page
        .locator('[data-feed-content]')
        .evaluate((node) => (node.scrollTop = 240));
      await page.evaluate(() => new Promise(requestAnimationFrame));
      const pinned = page
        .locator('[data-card-group-header][data-stuck]')
        .first();
      await expect(pinned).toBeVisible();
      const top = await page
        .locator('[data-feed-content]')
        .evaluate((node) => node.getBoundingClientRect().top);
      expect(Math.abs((await pinned.boundingBox())!.y - top)).toBeLessThan(1);
      await page.screenshot({ path: testInfo.outputPath('pinned-date.png') });
    }
    expect(
      await page.evaluate(
        () =>
          [
            ...document.querySelectorAll(
              '.media-row-rail [data-sidebar-resize]',
            ),
          ].flatMap((element) => element.getAnimations()).length,
      ),
    ).toBe(0);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.getByRole('button', { name: 'Collapse sidebar' }).click();
    await expect(page.locator('.primary-sidebar')).toHaveCSS('width', '72px');
    await testInfo.attach('sidebar-geometry.json', {
      body: JSON.stringify(evidence, null, 2),
      contentType: 'application/json',
    });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('sidebar motion stays grouped with a fully loaded YouTube feed', async ({
  page,
}, testInfo) => {
  const fixture = await installSidebarFixture(page);
  await page.goto('/');
  await expect(page.locator('[data-feed-content] [data-video-id]')).toHaveCount(
    60,
  );
  const feed = page.locator('[data-feed-content]');
  for (const count of [120, 144]) {
    await feed.evaluate((node) => (node.scrollTop = node.scrollHeight));
    await expect(
      page.locator('[data-feed-content] [data-video-id]'),
    ).toHaveCount(count);
  }
  await feed.evaluate((node) => (node.scrollTop = 240));
  await page.evaluate(() => document.fonts.ready);
  const results = [];
  for (const name of [
    'Collapse sidebar',
    'Close watchlists',
    'Expand sidebar',
    'Open watchlists',
  ]) {
    const result = await page.evaluate(async (name) => {
      const gaps: number[] = [];
      const longTasks: number[] = [];
      let animations = 0,
        cards = 0;
      const cardTransforms = new Set<string>();
      let gridLayout = 0;
      // Resizing can bring another row into view during the transition.
      const nearbyCards = new Set<Element>();
      const animatedCards = new Set<Element>();
      const collectNearbyCards = () => {
        for (const card of document.querySelectorAll(
          '[data-layout-key^="youtube-video:"]',
        )) {
          const rect = card.getBoundingClientRect();
          if (rect.bottom >= -240 && rect.top <= innerHeight + 240)
            nearbyCards.add(card);
        }
      };
      collectNearbyCards();
      const observer = new PerformanceObserver((list) =>
        longTasks.push(...list.getEntries().map((entry) => entry.duration)),
      );
      observer.observe({ type: 'longtask', buffered: false });
      let previous = performance.now();
      const started = previous;
      document
        .querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`)!
        .click();
      await new Promise<void>((resolve) => {
        const frame = (time: number) => {
          gaps.push(time - previous);
          previous = time;
          const active = document.getAnimations();
          collectNearbyCards();
          animations = Math.max(animations, active.length);
          const cardAnimations = active.filter(
            (animation) =>
              animation.effect instanceof KeyframeEffect &&
              (animation.effect.target as Element | null)?.matches(
                '[data-layout-key^="youtube-video:"]',
              ),
          );
          cards = Math.max(cards, cardAnimations.length);
          for (const animation of cardAnimations) {
            animatedCards.add(
              (animation.effect as KeyframeEffect).target as Element,
            );
            for (const keyframe of (
              animation.effect as KeyframeEffect
            ).getKeyframes())
              cardTransforms.add(
                String(keyframe.transform).replace(/\(.*/, ''),
              );
          }
          gridLayout += active.filter(
            (animation) =>
              animation.effect instanceof KeyframeEffect &&
              animation.effect
                .getKeyframes()
                .some((keyframe) => 'columnGap' in keyframe),
          ).length;
          if (time - started < 350) requestAnimationFrame(frame);
          else resolve();
        };
        requestAnimationFrame(frame);
      });
      observer.disconnect();
      return {
        name,
        gaps,
        longTasks,
        animations,
        cards,
        nearbyCards: nearbyCards.size,
        offscreenCards: [...animatedCards]
          .filter((card) => !nearbyCards.has(card))
          .map((card) => card.getAttribute('data-layout-key')),
        cardTransforms: [...cardTransforms],
        gridLayout,
      };
    }, name);
    // Cards move with their grid; later columns only get translate shifts.
    expect(result.cards).toBeLessThanOrEqual(result.nearbyCards);
    expect(result.offscreenCards).toEqual([]);
    expect(result.cardTransforms.every((name) => name === 'translate')).toBe(
      true,
    );
    // Media grids must stay on the compositor: no per-frame grid layout.
    expect(result.gridLayout).toBe(0);
    expect(result.animations).toBeLessThan(144);
    results.push(result);
  }
  await testInfo.attach('sidebar-performance.json', {
    body: JSON.stringify(results, null, 2),
    contentType: 'application/json',
  });
  await page.screenshot({ path: testInfo.outputPath('long-feed.png') });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
