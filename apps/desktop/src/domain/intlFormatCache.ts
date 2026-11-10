/**
 * Shared `Intl` formatters, one per locale and option set.
 *
 * Constructing an `Intl` formatter resolves the locale and loads its pattern
 * data every time, which costs far more than formatting with it. Timeline rows
 * format a timestamp on every render, so a formatter built per call shows up
 * as locale parsing in a renderer profile (#969). The key space is the handful
 * of option literals in the source times the UI locales, so it stays small.
 *
 * A formatter resolves the system time zone and default locale when it is
 * built, so the cache is dropped whenever the window regains focus: a laptop
 * that changed time zone while the app sat in the tray formats correctly again
 * without a restart.
 *
 * Every user-visible date, time, list, and number format goes through the
 * `ui*Format` helpers, which pin the locale to the Rust-resolved catalog
 * locale (#1477). Passing `undefined` would follow the OS locale instead and
 * mix languages when the app language differs from the system one.
 */
import { getActiveLocale, localeLanguageTag } from "../i18n/messages";

const dateTimeFormats = new Map<string, Intl.DateTimeFormat>();
const listFormats = new Map<string, Intl.ListFormat>();
const numberFormats = new Map<string, Intl.NumberFormat>();

function cacheKey(locale: string | undefined, options: object): string {
  return `${locale ?? ""}|${JSON.stringify(options)}`;
}

export function cachedDateTimeFormat(
  locale: string | undefined,
  options: Intl.DateTimeFormatOptions
): Intl.DateTimeFormat {
  const key = cacheKey(locale, options);
  let format = dateTimeFormats.get(key);
  if (!format) {
    format = new Intl.DateTimeFormat(locale, options);
    dateTimeFormats.set(key, format);
  }
  return format;
}

export function cachedListFormat(
  locale: string | undefined,
  options: Intl.ListFormatOptions
): Intl.ListFormat {
  const key = cacheKey(locale, options);
  let format = listFormats.get(key);
  if (!format) {
    format = new Intl.ListFormat(locale, options);
    listFormats.set(key, format);
  }
  return format;
}

export function cachedNumberFormat(
  locale: string | undefined,
  options: Intl.NumberFormatOptions
): Intl.NumberFormat {
  const key = cacheKey(locale, options);
  let format = numberFormats.get(key);
  if (!format) {
    format = new Intl.NumberFormat(locale, options);
    numberFormats.set(key, format);
  }
  return format;
}

/** The `Intl` locale of the selected catalog; the pseudo catalog formats as English. */
export function uiFormatLocale(): string {
  return localeLanguageTag(getActiveLocale());
}

export function uiDateTimeFormat(options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  return cachedDateTimeFormat(uiFormatLocale(), options);
}

export function uiListFormat(options: Intl.ListFormatOptions): Intl.ListFormat {
  return cachedListFormat(uiFormatLocale(), options);
}

export function uiNumberFormat(options: Intl.NumberFormatOptions = {}): Intl.NumberFormat {
  return cachedNumberFormat(uiFormatLocale(), options);
}

/**
 * A red unread badge, capped at `99+` (#1827, #1857). The full count belongs in
 * the badge's `title` and `aria-label`, not in the digits on screen.
 */
export function formatCappedUnread(count: number): string {
  return count > 99 ? "99+" : uiNumberFormat().format(count);
}

export function clearIntlFormatCache(): void {
  dateTimeFormats.clear();
  listFormats.clear();
  numberFormats.clear();
}

if (typeof window !== "undefined") {
  window.addEventListener("focus", clearIntlFormatCache);
}
