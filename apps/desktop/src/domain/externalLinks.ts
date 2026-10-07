/// The web-search provider is fixed and named in the localized action label:
/// opening the OS default browser does not discover that browser's own default
/// search engine, so the destination is Koushi's choice, not the browser's.
export const WEB_SEARCH_PROVIDER_LABEL = "DuckDuckGo";

/// Build the web-search URL for a selection (#1155).
///
/// The trimmed selection becomes one encoded query parameter; a whitespace-only
/// selection has no search target and returns `null`. Selected text can split an
/// emoji or contain malformed UTF-16, and `encodeURIComponent` throws on an
/// unpaired surrogate, so lone surrogates are replaced with U+FFFD first (the
/// same normalization as `String.prototype.toWellFormed`), never before the
/// exact-text Copy path.
export function webSearchUrl(query: string): string | null {
  const trimmed = query.trim();
  if (!trimmed) {
    return null;
  }
  return `https://duckduckgo.com/?q=${encodeURIComponent(toWellFormed(trimmed))}`;
}

function toWellFormed(value: string): string {
  let wellFormed = "";
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        wellFormed += value.slice(index, index + 2);
        index += 1;
      } else {
        wellFormed += "\uFFFD";
      }
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      wellFormed += "\uFFFD";
    } else {
      wellFormed += value[index];
    }
  }
  return wellFormed;
}

export function toExternalHttpUrl(rawUrl: string | null | undefined): string | null {
  if (!rawUrl) {
    return null;
  }
  try {
    const url = new URL(rawUrl);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return null;
    }
    return url.toString();
  } catch {
    return null;
  }
}
