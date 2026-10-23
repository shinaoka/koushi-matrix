import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, test, vi } from "vitest";
import katex from "katex";

import type { TimelineFormattedBody } from "../../domain/coreEvents";
import { renderFormattedBody, renderPlainTextBody } from "./TimelineMessageBody";

const NO_SPOILERS = { revealed: new Set<string>(), reveal: () => undefined };

const renderPlain = (text: string, room: boolean) =>
  renderToStaticMarkup(
    renderPlainTextBody(text, [], undefined, [], {}, NO_SPOILERS, undefined, {
      userIds: new Set<string>(),
      room
    })
  );

const GIANT_RULE_SOURCE = String.raw`\rule{1000em}{1000em}`;
const UNCLAMPED_GIANT_RULE_MARKUP = katex.renderToString(GIANT_RULE_SOURCE, {
  displayMode: false,
  strict: false,
  throwOnError: false,
  trust: false,
  maxExpand: 1000
});

const renderMath = (
  tag: "span" | "div",
  source: string,
  fallbackText = source
) =>
  renderToStaticMarkup(
    renderFormattedBody(
      {
        html: `<${tag} data-mx-maths="${source}">${fallbackText}</${tag}>`,
        plain_text: fallbackText,
        code_blocks: []
      } satisfies TimelineFormattedBody,
      [],
      false,
      () => undefined,
      [],
      { revealed: new Set<string>(), reveal: () => undefined },
      undefined
    )
  );

afterEach(() => {
  vi.restoreAllMocks();
});

describe("renderPlainTextBody room mention pills (#1123)", () => {
  test("draws the pill for the spec-standard Element room mention", () => {
    const markup = renderPlain("@room heads up", true);

    expect(markup).toContain('class="message-mention-pill"');
    expect(markup).toContain('data-mention-room="true"');
  });

  test("leaves plain @room text alone without the event flag", () => {
    const markup = renderPlain("@room heads up", false);

    expect(markup).not.toContain("message-mention-pill");
  });

  test("pills only a whole room token, not a suffix of another word", () => {
    const markup = renderPlain("see me@room.example and @room", true);

    expect(markup.match(/message-mention-pill/g)).toHaveLength(1);
  });
});

describe("renderFormattedBody math bounds", () => {
  test("rejects 1025-unit inline and display expressions without calling KaTeX", () => {
    const renderToString = vi.spyOn(katex, "renderToString");
    const source = "x".repeat(1025);

    const inlineMarkup = renderMath("span", source, "inline fallback");
    const displayMarkup = renderMath("div", source, "display fallback");

    expect(inlineMarkup).toContain("inline fallback");
    expect(displayMarkup).toContain("display fallback");
    expect(inlineMarkup).not.toContain('class="katex"');
    expect(displayMarkup).not.toContain('class="katex"');
    expect(renderToString).not.toHaveBeenCalled();
  });

  test("admits an exact 1024-unit source after trimming outer whitespace", () => {
    const renderToString = vi.spyOn(katex, "renderToString");
    const source = "x".repeat(1024);

    const markup = renderMath("span", ` \t${source}\n `);

    expect(markup).toContain('class="message-math"');
    expect(markup).toContain('class="katex"');
    expect(markup).toContain(`data-mx-maths="${source}"`);
    expect(renderToString).toHaveBeenCalledTimes(1);
    expect(renderToString).toHaveBeenCalledWith(
      source,
      expect.objectContaining({
        displayMode: false,
        strict: false,
        throwOnError: false,
        trust: false,
        maxExpand: 1000,
        maxSize: 20
      })
    );
  });

  test("keeps ordinary inline and display formulas with finite KaTeX options", () => {
    const renderToString = vi.spyOn(katex, "renderToString");

    const inlineMarkup = renderMath("span", "E=mc^2");
    const displayMarkup = renderMath("div", "E=mc^2");

    expect(inlineMarkup).toContain('class="message-math"');
    expect(inlineMarkup).toContain('class="katex"');
    expect(inlineMarkup).toContain('data-mx-maths="E=mc^2"');
    expect(displayMarkup).toContain('class="message-math is-block"');
    expect(displayMarkup).toContain('class="katex"');
    expect(displayMarkup).toContain('data-mx-maths="E=mc^2"');
    expect(renderToString).toHaveBeenNthCalledWith(
      1,
      "E=mc^2",
      expect.objectContaining({
        displayMode: false,
        strict: false,
        throwOnError: false,
        trust: false,
        maxExpand: 1000,
        maxSize: 20
      })
    );
    expect(renderToString).toHaveBeenNthCalledWith(
      2,
      "E=mc^2",
      expect.objectContaining({
        displayMode: true,
        strict: false,
        throwOnError: false,
        trust: false,
        maxExpand: 1000,
        maxSize: 20
      })
    );
  });

  test("keeps empty and exception fallbacks visible", () => {
    const renderToString = vi.spyOn(katex, "renderToString");

    const emptyMarkup = renderMath("span", "   ", "empty fallback");
    expect(emptyMarkup).toContain("empty fallback");
    expect(emptyMarkup).not.toContain('class="katex"');
    expect(renderToString).not.toHaveBeenCalled();

    renderToString.mockImplementation(() => {
      throw new Error("synthetic KaTeX failure");
    });
    const exceptionMarkup = renderMath("div", "E=mc^2", "exception fallback");
    expect(exceptionMarkup).toContain("exception fallback");
    expect(exceptionMarkup).not.toContain('class="katex"');
    expect(renderToString).toHaveBeenCalledTimes(1);
  });

  test("clamps the installed KaTeX giant-rule dimensions to 20em", () => {
    expect(UNCLAMPED_GIANT_RULE_MARKUP).toContain('style="height:1000em;');
    expect(UNCLAMPED_GIANT_RULE_MARKUP).toContain("border-right-width:1000em;");

    const renderToString = vi.spyOn(katex, "renderToString");
    const markup = renderMath("span", GIANT_RULE_SOURCE);

    expect(renderToString).toHaveBeenCalledTimes(1);
    expect(markup).toContain('style="height:20em;');
    expect(markup).toContain("border-right-width:20em;");
    expect(markup).not.toContain('style="height:1000em;');
    expect(markup).not.toContain("border-right-width:1000em;");
  });

  test("rejects twenty approximately 2950-unit attack expressions with zero KaTeX calls", () => {
    const renderToString = vi.spyOn(katex, "renderToString");
    const controlMarkup = renderMath("span", "E=mc^2");
    expect(controlMarkup).toContain('class="katex"');
    expect(renderToString).toHaveBeenCalledTimes(1);
    renderToString.mockClear();

    const attackSource = "x".repeat(2950);
    const attackHtml = Array.from(
      { length: 20 },
      (_, index) =>
        `<span data-mx-maths="${attackSource}">attack fallback ${index}</span>`
    ).join("");

    renderToString.mockImplementation(() => {
      throw new Error("sentinel: oversized expression reached KaTeX");
    });
    const markup = renderToStaticMarkup(
      renderFormattedBody(
        {
          html: attackHtml,
          plain_text: "attack fallback",
          code_blocks: []
        } satisfies TimelineFormattedBody,
        [],
        false,
        () => undefined,
        [],
        { revealed: new Set<string>(), reveal: () => undefined },
        undefined
      )
    );

    expect(markup).toContain("attack fallback 0");
    expect(markup).toContain("attack fallback 19");
    expect(markup).not.toContain('class="katex"');
    expect(renderToString).not.toHaveBeenCalled();
  });
});

describe("renderFormattedBody mention pills (#874, #1123)", () => {
  const mentionHtml =
    '<a href="https://matrix.to/#/%40alice%3Aexample.test">@Alice</a> and <a href="https://matrix.to/#/%40bob%3Aexample.test">@Bob</a>';

  const render = (userIds: ReadonlySet<string>, room = false) =>
    renderToStaticMarkup(
      renderFormattedBody(
        {
          html: mentionHtml,
          plain_text: "@Alice and @Bob",
          code_blocks: []
        } satisfies TimelineFormattedBody,
        [],
        false,
        () => undefined,
        [],
        { revealed: new Set<string>(), reveal: () => undefined },
        undefined,
        { userIds, room }
      )
    );

  test("draws the pill only for the user the message mentions", () => {
    const markup = render(new Set(["@alice:example.test"]));

    expect(markup).toContain('class="message-mention-pill"');
    expect(markup).toContain('data-mention-user-id="@alice:example.test"');
    // The unmentioned permalink stays an ordinary link.
    expect(markup).not.toContain('data-mention-user-id="@bob:example.test"');
    expect(markup.match(/message-mention-pill/g)).toHaveLength(1);
  });

  // #1328: Element writes the anchor text without `@` (`>Alice<`), while the
  // composer's chip and the plain-body pill both read `@Alice`. The pill adds the
  // prefix itself, and a label that already carries one is not doubled. The
  // event's HTML and the copied plain text keep the sender's spelling.
  test("spells an Element-shaped mention anchor like the composer's chip", () => {
    const renderAnchor = (label: string) =>
      renderToStaticMarkup(
        renderFormattedBody(
          {
            html: `<a href="https://matrix.to/#/%40alice%3Aexample.test">${label}</a> please look`,
            plain_text: `${label} please look`,
            code_blocks: []
          } satisfies TimelineFormattedBody,
          [],
          false,
          () => undefined,
          [],
          { revealed: new Set<string>(), reveal: () => undefined },
          undefined,
          { userIds: new Set(["@alice:example.test"]), room: false }
        )
      );

    const element = renderAnchor("Alice");
    expect(element).toContain(">@Alice</a>");
    expect(renderAnchor("@Alice")).toContain(">@Alice</a>");
    expect(renderAnchor("@Alice")).not.toContain(">@@Alice</a>");
  });

  test("draws no pill for a message that mentions nobody", () => {
    const markup = render(new Set());

    expect(markup).not.toContain("message-mention-pill");
    // Both anchors remain clickable links.
    expect(markup.match(/<a /g)).toHaveLength(2);
  });

  // #1123 boundary: the formatted renderer recognizes mentions exactly as it did
  // before this change — user mentions through their matrix.to anchors and
  // nothing else — so a bare room token in a formatted body is not pilled, with
  // or without the flag. Pilling text nodes here would also have to resolve
  // boundaries across sibling elements and would pill inside inline code.
  const renderRoomMention = (html: string, plainText: string, room: boolean) =>
    renderToStaticMarkup(
      renderFormattedBody(
        { html, plain_text: plainText, code_blocks: [] } satisfies TimelineFormattedBody,
        [],
        false,
        () => undefined,
        [],
        { revealed: new Set<string>(), reveal: () => undefined },
        undefined,
        { userIds: new Set<string>(), room }
      )
    );

  test("leaves a bare room token in a formatted body alone", () => {
    expect(renderRoomMention("@room heads up", "@room heads up", true)).not.toContain(
      "message-mention-pill"
    );
    expect(renderRoomMention("@room heads up", "@room heads up", false)).not.toContain(
      "message-mention-pill"
    );
  });
});

describe("renderFormattedBody linkify beside existing anchors (#1542)", () => {
  type Range = { url: string; start_utf16: number; end_utf16: number };
  const rangeOf = (plainText: string, url: string): Range => {
    const start = plainText.indexOf(url);
    if (start < 0) throw new Error(`fixture: ${url} not in plain text`);
    return { url, start_utf16: start, end_utf16: start + url.length };
  };
  const renderLinked = (
    html: string,
    plainText: string,
    ranges: Range[],
    userIds: ReadonlySet<string> = new Set<string>()
  ) =>
    renderToStaticMarkup(
      renderFormattedBody(
        { html, plain_text: plainText, code_blocks: [] } satisfies TimelineFormattedBody,
        ranges,
        false,
        () => undefined,
        [],
        NO_SPOILERS,
        undefined,
        { userIds, room: false }
      )
    );
  const anchorCount = (markup: string) => (markup.match(/<a\b/g) ?? []).length;
  const NESTED_ANCHOR = /<a\b(?:(?!<\/a>)[\s\S])*<a\b/;

  test("links a bare URL that follows a mention anchor and keeps the pill", () => {
    const url = "https://arxiv.org/abs/2610.22307";
    const html = `<a href="https://matrix.to/#/%40a%3Aexample.test">a</a>: text ${url}`;
    const plainText = `a: text ${url}`;
    const markup = renderLinked(
      html,
      plainText,
      [rangeOf(plainText, url)],
      new Set(["@a:example.test"])
    );

    expect(markup).toContain(`href="${url}"`);
    expect(markup).toContain('data-mention-user-id="@a:example.test"');
  });

  test("does not nest an anchor inside an authored link to the same URL", () => {
    const url = "https://example.com";
    const markup = renderLinked(`<a href="${url}">${url}</a>`, url, [rangeOf(url, url)]);

    expect(anchorCount(markup)).toBe(1);
    expect(markup).not.toMatch(NESTED_ANCHOR);
  });

  test("links only the bare URL when an authored link and a bare URL coexist", () => {
    const linked = "https://example.com/a";
    const bare = "https://example.com/b";
    const html = `see <a href="${linked}">${linked}</a> and ${bare}`;
    const plainText = `see ${linked} and ${bare}`;
    const markup = renderLinked(html, plainText, [
      rangeOf(plainText, linked),
      rangeOf(plainText, bare)
    ]);

    expect(anchorCount(markup)).toBe(2);
    expect(markup).toContain(`href="${bare}"`);
    expect(markup).not.toMatch(NESTED_ANCHOR);
  });

  test("leaves URLs inside inline code unlinked", () => {
    const url = "https://x.example/y";
    const html = `run <code>${url}</code>`;
    const plainText = `run ${url}`;
    const markup = renderLinked(html, plainText, [rangeOf(plainText, url)]);

    expect(anchorCount(markup)).toBe(0);
  });

  test("drops a range whose text does not match its URL instead of mislinking", () => {
    const url = "https://example.com/real";
    const html = `<a href="https://matrix.to/#/%40a%3Aexample.test">a</a> ${url}`;
    const plainText = `a ${url}`;
    const shifted = { url, start_utf16: 1, end_utf16: 1 + url.length };
    const markup = renderLinked(html, plainText, [shifted]);

    expect(markup).not.toContain(`href="${url}"`);
  });

  test("still links when the plain text carries separators the HTML lacks", () => {
    const url = "https://example.invalid/pull/7";
    const html = `<a href="https://matrix.to/#/%40alice%3Aexample.invalid">@Alice</a><br>${url}<br><br>Feedback`;
    const plainText = `@Alice\n${url}\n\nFeedback`;
    const markup = renderLinked(html, plainText, [rangeOf(plainText, url)]);

    expect(markup).toContain(`href="${url}"`);
  });

  // #1596: the Rust projection detects links on block boundaries and maps the
  // ranges back onto the concatenated plain text, so adjacent list items publish
  // one exact range per URL. The renderer must link both without nesting.
  test("links both URLs of adjacent list items from Rust-shaped ranges", () => {
    const first = "https://example.invalid/profiles/member_one?locale=ja";
    const second = "https://example.invalid/profiles/member_two?locale=ja";
    const html = `<ul><li>Member 1: ${first}</li><li>Member 2: ${second}</li></ul>`;
    const plainText = `Member 1: ${first}Member 2: ${second}`;
    const markup = renderLinked(html, plainText, [
      rangeOf(plainText, first),
      rangeOf(plainText, second)
    ]);

    expect(markup).toContain(`href="${first}"`);
    expect(markup).toContain(`href="${second}"`);
    expect(anchorCount(markup)).toBe(2);
    expect(markup).not.toMatch(NESTED_ANCHOR);
  });
});
