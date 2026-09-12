import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, test, vi } from "vitest";
import katex from "katex";

import type { TimelineFormattedBody } from "../../domain/coreEvents";
import { renderFormattedBody } from "./TimelineMessageBody";

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

describe("renderFormattedBody mention pills (#874)", () => {
  const mentionHtml =
    '<a href="https://matrix.to/#/%40alice%3Aexample.test">@Alice</a> and <a href="https://matrix.to/#/%40bob%3Aexample.test">@Bob</a>';

  const render = (mentionedUserIds: ReadonlySet<string>) =>
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
        mentionedUserIds
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

  test("draws no pill for a message that mentions nobody", () => {
    const markup = render(new Set());

    expect(markup).not.toContain("message-mention-pill");
    // Both anchors remain clickable links.
    expect(markup.match(/<a /g)).toHaveLength(2);
  });
});
