// Repeated layouts shared by library, account and administration views.
export const panelClass =
  'panel mb-5 min-w-0 rounded-none border border-line bg-surface p-6 shadow-none compact:p-4 settings-panel:mb-6 settings-panel:border-0 settings-panel:border-b settings-panel:border-line settings-panel:bg-transparent settings-panel:p-0 settings-panel:pb-6 settings-panel:compact:p-4';

export const inlineFormClass =
  'my-4.5 flex flex-wrap items-end gap-3 [&_label]:m-0 [&_label]:min-w-[min(150px,100%)] [&_label]:flex-1 compact:flex-col compact:items-stretch compact:[&_label]:w-full';

export const rowClass =
  'row flex items-center justify-between gap-4 border-t border-line py-3.25 text-xs leading-normal [&>div]:min-w-0 [&>div]:wrap-anywhere [&>span]:min-w-0 [&>span]:wrap-anywhere [&_small]:mt-1 compact:flex-wrap compact:gap-2';

export const settingRowClass =
  'mb-4.5 flex items-center justify-between gap-4 border-b border-line py-4 [&_p]:mt-1.25 [&_p]:mb-0 compact:flex-wrap compact:items-start';

export const badgeClass =
  'inline-flex items-center border border-line bg-surface-soft px-1.75 py-0.75 text-[9px] tracking-[0.06em] text-muted uppercase';

// Keep native inputs so Svelte preserves number, file and select bindings.
export const formControlClass =
  'w-full min-w-0 rounded-none border border-line bg-surface-strong px-2.75 py-2.25 text-foreground placeholder:text-muted placeholder:opacity-80 focus:border-line-strong focus:outline-accent disabled:cursor-not-allowed disabled:opacity-46 [&[type=checkbox]]:size-3.75 [&[type=checkbox]]:shrink-0 [&[type=checkbox]]:accent-accent [&[type=radio]]:size-3.75 [&[type=radio]]:shrink-0 [&[type=radio]]:accent-accent';

const eyebrowTypographyClass =
  'font-[650] tracking-[0.17em] text-muted uppercase';
export const eyebrowTextClass = `text-[0.625rem] ${eyebrowTypographyClass}`;
export const eyebrowClass = `mb-2 text-[0.75rem] ${eyebrowTypographyClass}`;

export const emptyClass =
  'flex min-h-65 flex-col items-center justify-center border border-line bg-surface p-8 text-center text-muted [&_svg]:mb-5 [&_svg]:text-accent [&_h2]:text-foreground [&_p]:max-w-100 [&_p]:text-[0.75rem]';

export const statsClass =
  'my-5 grid grid-cols-[repeat(auto-fit,minmax(135px,1fr))] gap-3 [&>div]:border [&>div]:border-line [&>div]:bg-surface-soft [&>div]:p-5 [&_strong]:text-2xl [&_strong]:leading-normal [&_strong]:font-medium [&_strong]:tracking-[-0.04em] [&_small]:mt-1.5 [&_small]:text-[10px] [&_small]:tracking-[0.05em] [&_small]:uppercase';
