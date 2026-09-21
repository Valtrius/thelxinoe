export const settingsPanelClass =
  'min-w-0 border-t border-line py-4 first:border-t-0 first:pt-0 last:pb-0';
export const panelHeadingClass =
  'mb-4 flex items-center justify-between gap-4 [&_h3]:text-sm';
export const fieldGroupClass =
  'grid gap-[0.4rem] text-[0.7rem] font-semibold tracking-[0.04em] text-foreground [&>p]:text-[0.62rem] [&>p]:leading-[1.15rem] [&>p]:font-normal [&>p]:text-muted';
export const controlClass =
  'min-h-9 w-full border border-line bg-surface-soft px-[0.65rem] py-[0.45rem] text-xs text-foreground focus:border-line-strong';
export const selectionButtonClass = (selected: boolean) =>
  `flex cursor-pointer items-center gap-2 border px-3 py-2 text-left disabled:cursor-default disabled:opacity-45 ${selected ? 'border-line-strong bg-accent-soft' : 'border-line bg-surface-soft'}`;
export const dataStatClass =
  'border border-line border-l-line-strong bg-surface-soft p-[0.7rem] shadow-panel [&>span]:block [&>span]:text-[0.56rem] [&>span]:tracking-[0.08em] [&>span]:text-muted [&>span]:uppercase [&>strong]:mt-[0.2rem] [&>strong]:block [&>strong]:text-base [&>strong]:font-medium';
