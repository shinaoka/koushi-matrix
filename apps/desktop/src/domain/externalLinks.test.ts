import { describe, expect, it } from "vitest";

import { toExternalHttpUrl, webSearchUrl } from "./externalLinks";

describe("externalLinks", () => {
  it("normalizes http URLs", () => {
    expect(toExternalHttpUrl("https://example.com/page")).toBe("https://example.com/page");
    expect(toExternalHttpUrl("http://example.com/path?q=1")).toBe(
      "http://example.com/path?q=1"
    );
  });

  it("rejects non-http and malformed URLs", () => {
    expect(toExternalHttpUrl("javascript:alert(1)")).toBeNull();
    expect(toExternalHttpUrl("file:///tmp/secret.txt")).toBeNull();
    expect(toExternalHttpUrl("not a URL")).toBeNull();
    expect(toExternalHttpUrl(null)).toBeNull();
  });
});

describe("webSearchUrl", () => {
  it("encodes the trimmed selection as a single query parameter", () => {
    expect(webSearchUrl("Second phrase.")).toBe(
      "https://duckduckgo.com/?q=Second%20phrase."
    );
    // Japanese, emoji, `&`, `#`, `+` and newlines must survive as one query
    // value: none of them may split or terminate the parameter.
    expect(webSearchUrl(" こんにちは 👩‍👩‍👦\na&b#c+d ")).toBe(
      `https://duckduckgo.com/?q=${encodeURIComponent("こんにちは 👩‍👩‍👦\na&b#c+d")}`
    );
    const url = webSearchUrl("a&b#c+d");
    expect(url).not.toBeNull();
    expect(new URL(url ?? "").searchParams.get("q")).toBe("a&b#c+d");
  });

  it("uses the named provider and never leaks the raw selection", () => {
    const url = webSearchUrl("plain text");
    expect(url).toBe("https://duckduckgo.com/?q=plain%20text");
    expect(toExternalHttpUrl(url)).toBe(url);
  });

  it("has no search target for an empty or whitespace-only selection", () => {
    expect(webSearchUrl("")).toBeNull();
    expect(webSearchUrl("   \n\t ")).toBeNull();
  });

  it("does not throw on an unpaired surrogate produced by splitting an emoji", () => {
    // A range that splits a surrogate pair hands over one unpaired surrogate.
    const highOnly = "\ud83d";
    expect(() => webSearchUrl(`x${highOnly}y`)).not.toThrow();
    expect(webSearchUrl(`x${highOnly}y`)).toBe(
      "https://duckduckgo.com/?q=x%EF%BF%BDy"
    );
    const lowOnly = "\udc69";
    expect(() => webSearchUrl(`x${lowOnly}y`)).not.toThrow();
    expect(webSearchUrl(`x${lowOnly}y`)).toBe(
      "https://duckduckgo.com/?q=x%EF%BF%BDy"
    );
  });
});
