// Inline timeline search highlighting must obey the SAME matching rule as the
// Rust search matcher (koushi-search `exact_range` / `normalize_cjk_search_text`
// = NFKC + case fold) so a visible highlight and the Search panel's exact-match
// count never disagree (issue #162). This mirrors the Rust rule on the JS side.

/** NFKC + case fold, matching Rust `normalize_cjk_search_text`. */
export function normalizeForSearch(value: string): string {
  return value.normalize("NFKC").toLowerCase();
}

/**
 * Locate the UTF-16 `[start, end)` range in `text` matching `query` under the
 * NFKC + case-fold rule, or `null` if absent. Normalization can change string
 * length (e.g. `ﬁ` → `fi`), so a per-code-point map back to original offsets is
 * kept — the same idea as the Rust `NormalizedHaystack` source-range map.
 */
export function findQueryHighlightRange(
  text: string,
  query: string
): { start: number; end: number } | null {
  const needle = normalizeForSearch(query.trim());
  if (!needle) {
    return null;
  }

  let normalized = "";
  // For each normalized UTF-16 unit, the original text offset it came from.
  const startOffsets: number[] = [];
  const endOffsets: number[] = [];
  let original = 0;
  for (const codePoint of text) {
    const normalizedCp = normalizeForSearch(codePoint);
    const codePointLength = codePoint.length; // 1 or 2 UTF-16 units
    for (let i = 0; i < normalizedCp.length; i += 1) {
      normalized += normalizedCp[i];
      startOffsets.push(original);
      endOffsets.push(original + codePointLength);
    }
    original += codePointLength;
  }

  const index = normalized.indexOf(needle);
  if (index < 0) {
    return null;
  }
  return {
    start: startOffsets[index],
    end: endOffsets[index + needle.length - 1],
  };
}
