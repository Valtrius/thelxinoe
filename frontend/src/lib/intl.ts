// Creating a formatter costs far more than formatting with one, and lists
// format a date or a count for every row: each is created once.
const formatters = new Map<string, unknown>();

function cached<T>(key: unknown[], create: () => T): T {
  const id = JSON.stringify(key);
  let formatter = formatters.get(id) as T | undefined;
  if (!formatter) formatters.set(id, (formatter = create()));
  return formatter;
}

export const dateFormat = (
  locale: string | undefined,
  options: Intl.DateTimeFormatOptions,
) =>
  cached(
    ['date', locale, options],
    () => new Intl.DateTimeFormat(locale, options),
  );

export const relativeTimeFormat = (options: Intl.RelativeTimeFormatOptions) =>
  cached(
    ['relative', options],
    () => new Intl.RelativeTimeFormat(undefined, options),
  );

export const numberFormat = (options: Intl.NumberFormatOptions) =>
  cached(['number', options], () => new Intl.NumberFormat(undefined, options));
