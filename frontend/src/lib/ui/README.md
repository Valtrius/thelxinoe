# Shared UI

Use Tailwind utilities for component styling. Keep theme tokens and global
typography in `src/app.css`; put component layout and states with the component.
Inline styles are for values calculated at runtime, such as progress, crop
coordinates, chart data, and measured animation geometry.

- `Button` provides the application actions, including player and compact
  credential buttons. Use a Bits UI `child` snippet to compose it with a menu
  trigger and forward the trigger's props.
- `Panel` and `SectionHeading` provide the common section and heading layouts.
- `FormField` renders a label around its content. Keep the native input, select,
  or textarea inside it and apply `formControlClass`, so Svelte retains native
  numeric values, file bindings, browser validation, and label association. Use
  `twMerge(formControlClass, extraClasses)` for overrides.
- `AutoSaveForm` owns the save state, validation, disabled fieldset, and retry
  behavior for preferences that save on change.
- `Notice` provides callouts and error boxes. Set the appropriate `role` at the
  call site when an update should be announced; static notices need no live role.
- `StatusIndicator` provides service state text with a colored dot, including
  activity and reduced-motion behavior.
- `ProgressBar` provides determinate and indeterminate progress. Pass a `label`
  for an accessible progress bar; omit it for decorative media overlays that
  already have a text description. `barClass` styles the moving fill.

The shell's inclusive breakpoints are declared from wide to narrow in
`app.css`: `service-narrow` (1050px), `narrow` (1000px), `compact` (720px),
and `tight` (440px). Use these together for overlapping rules so each narrower
layout wins at its boundary. Provider views retain their own `sm`/`md`/`lg`
breakpoints. Mixing custom and arbitrary breakpoint variants can change their
override order.

Add component options for concrete callers. Keep domain actions and API requests
in the owning feature instead of adding them to visual primitives.
