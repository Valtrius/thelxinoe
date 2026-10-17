<script lang="ts">
  import type { Snippet } from 'svelte';
  import { ChevronRight } from '@lucide/svelte';
  import {
    followLink,
    routeHref,
    sectionRoute,
    type SettingsSection,
  } from '../navigation';
  import { serviceIcons } from '../services/icons';
  import SettingsCard from '../ui/SettingsCard.svelte';
  import { filterSettingsCards } from '../ui/settings-card-filter';
  import { eyebrowTextClass } from '../ui/styles';
  import type { SettingsSearchResult } from './search';

  // One page in the search results: a link to the page, then the page itself
  // with only its matching cards, or links into the media services page.
  let {
    result,
    open,
    children,
  }: {
    result: SettingsSearchResult;
    open: (section: SettingsSection, service?: string) => void;
    children?: Snippet;
  } = $props();
  filterSettingsCards((title) => result.cards.includes(title));
  const href = (service?: string) =>
    routeHref({ ...sectionRoute('Settings', result.page.id), service });
  const follow = (event: MouseEvent, service?: string) => {
    if (!followLink(event)) return;
    event.preventDefault();
    open(result.page.id, service);
  };
</script>

<section
  aria-label={`${result.page.label} results`}
  class="col-span-full grid min-w-0 gap-3"
>
  <a
    href={href()}
    class={[
      eyebrowTextClass,
      'flex w-fit items-center gap-2 transition-colors hover:text-accent',
    ]}
    onclick={(event) => follow(event)}
  >
    <result.page.icon size={14} strokeWidth={1.8} aria-hidden="true" />
    {result.page.label}
    <ChevronRight size={12} aria-hidden="true" />
  </a>
  <div class="settings-grid settings-results">
    {#if result.links}
      <SettingsCard>
        {#each result.links as link (link.service ?? '')}
          <a
            href={href(link.service)}
            class="flex min-w-0 items-center gap-3 px-5.5 py-3.25 transition-colors hover:bg-surface-soft compact:px-4"
            onclick={(event) => follow(event, link.service)}
          >
            {#if link.service}<img
                src={serviceIcons[link.service]}
                alt=""
                class="size-6 shrink-0 object-contain"
              />{:else}<result.page.icon
                size={20}
                strokeWidth={1.6}
                class="size-6 shrink-0 text-muted"
                aria-hidden="true"
              />{/if}
            <span class="min-w-0 flex-1">
              <span class="block text-[13px] leading-snug font-[550]"
                >{link.label}</span
              >
              <span class="mt-0.75 block text-xs leading-relaxed text-muted"
                >{link.description}</span
              >
            </span>
            <ChevronRight
              size={14}
              class="shrink-0 text-muted"
              aria-hidden="true"
            />
          </a>
        {/each}
      </SettingsCard>
    {:else}
      {@render children?.()}
    {/if}
  </div>
</section>
