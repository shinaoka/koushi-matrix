import { Fragment, type ReactNode } from "react";
import { Copy } from "lucide-react";
import katex from "katex";

import { t } from "../../i18n/messages";
import { openExternalHttpUrl } from "../../backend/linkMediaRuntime";
import { toExternalHttpUrl } from "../../domain/externalLinks";
import { parseMatrixPermalink, type MatrixPermalinkTarget } from "../../domain/matrixPermalink";
import type { TimelineItem, TimelineLinkRange } from "../../domain/coreEvents";
import type { TextRange, UserProfile } from "../../domain/types";
import type { TimelineRowActionHandlers } from "./TimelineItemRow";

type TimelineMentionToken = {
  token: string;
  userId: string;
};

/**
 * Issue #874: a mention pill is a property of the message, not of the viewer's
 * loaded profiles. Only ids the event's `m.mentions` named are pill candidates.
 */
const NO_MENTIONED_USER_IDS: ReadonlySet<string> = new Set<string>();

/** Opens a Matrix entity a rendered message links to. */
export type OpenMatrixTargetHandler = (target: MatrixPermalinkTarget) => void;

/**
 * Decide what activating a rendered message link does.
 *
 * A `matrix.to` URL names a room or user this client already owns, so it is
 * navigation rather than a web link: handing it to the browser would bounce the
 * user out of the app to reach something the app can open directly. Every other
 * URL stays an ordinary external link. Both message renderers route through
 * here so plain-text and formatted links cannot drift apart.
 */
function activateTimelineLink(
  href: string,
  onOpenMatrixTarget: OpenMatrixTargetHandler | undefined
): void {
  const target = onOpenMatrixTarget ? parseMatrixPermalink(href) : null;
  if (target && onOpenMatrixTarget) {
    onOpenMatrixTarget(target);
    return;
  }
  void openExternalHttpUrl(href);
}

export function renderTimelineMessageText(
  text: string,
  highlightRanges: TextRange[] = [],
  profileUsers: Record<string, UserProfile> = {},
  baseOffset = 0,
  mentionedUserIds: ReadonlySet<string> = NO_MENTIONED_USER_IDS
) {
  const mentionTokens = timelineMentionTokens(profileUsers, mentionedUserIds);
  let offset = baseOffset;
  return text.split("\n").map((line, index) => {
    const lineOffset = offset;
    offset += line.length + 1;
    return (
      <span key={`${line}:${index}`}>
        {index > 0 ? <br /> : null}
        {renderTimelineMessageLine(line, highlightRanges, mentionTokens, lineOffset)}
      </span>
    );
  });
}

function renderTimelineMessageTextWithSpoilers(
  text: string,
  spoilerSpans: TimelineItem["spoiler_spans"] | undefined,
  highlightRanges: TextRange[],
  profileUsers: Record<string, UserProfile>,
  spoilerState: SpoilerRevealState,
  mentionedUserIds: ReadonlySet<string>
): ReactNode {
  const spans = normalizeSpoilerSpans(spoilerSpans, text.length);
  if (spans.length === 0) {
    return renderTimelineMessageText(text, highlightRanges, profileUsers, 0, mentionedUserIds);
  }

  const nodes: ReactNode[] = [];
  let cursor = 0;
  for (const [index, span] of spans.entries()) {
    if (span.start_utf16 > cursor) {
      const visibleText = text.slice(cursor, span.start_utf16);
      nodes.push(
        <Fragment key={`text:${cursor}`}>
          {renderTimelineMessageText(
            visibleText,
            highlightRanges,
            profileUsers,
            cursor,
            mentionedUserIds
          )}
        </Fragment>
      );
    }

    const spoilerText = text.slice(span.start_utf16, span.end_utf16);
    nodes.push(
      renderSpoiler(
        `plain:${span.start_utf16}:${span.end_utf16}:${index}`,
        renderTimelineMessageText(
          spoilerText,
          highlightRanges,
          profileUsers,
          span.start_utf16,
          mentionedUserIds
        ),
        span.reason,
        spoilerState
      )
    );
    cursor = span.end_utf16;
  }

  if (cursor < text.length) {
    nodes.push(
      <Fragment key={`text:${cursor}`}>
        {renderTimelineMessageText(
          text.slice(cursor),
          highlightRanges,
          profileUsers,
          cursor,
          mentionedUserIds
        )}
      </Fragment>
    );
  }
  return nodes;
}

export function renderPlainTextBody(
  text: string,
  linkRanges: TimelineLinkRange[],
  spoilerSpans: TimelineItem["spoiler_spans"] | undefined,
  highlightRanges: TextRange[],
  profileUsers: Record<string, UserProfile>,
  spoilerState: SpoilerRevealState,
  onOpenMatrixTarget: OpenMatrixTargetHandler | undefined,
  mentionedUserIds: ReadonlySet<string> = NO_MENTIONED_USER_IDS
): ReactNode {
  if (linkRanges.length === 0) {
    return renderTimelineMessageTextWithSpoilers(
      text,
      spoilerSpans,
      highlightRanges,
      profileUsers,
      spoilerState,
      mentionedUserIds
    );
  }
  const spans = normalizeSpoilerSpans(spoilerSpans, text.length);
  const sortedLinks = [...linkRanges].sort(
    (left, right) => left.start_utf16 - right.start_utf16
  );

  const nodes: ReactNode[] = [];
  let cursor = 0;
  for (const [index, span] of spans.entries()) {
    if (span.start_utf16 > cursor) {
      nodes.push(
        <Fragment key={`text:${cursor}`}>
          {renderPlainTextSegment(
            text,
            cursor,
            span.start_utf16,
            sortedLinks,
            highlightRanges,
            profileUsers,
            onOpenMatrixTarget,
            mentionedUserIds
          )}
        </Fragment>
      );
    }

    const spoilerText = renderPlainTextSegment(
      text,
      span.start_utf16,
      span.end_utf16,
      sortedLinks,
      highlightRanges,
      profileUsers,
      onOpenMatrixTarget,
      mentionedUserIds
    );
    nodes.push(
      renderSpoiler(
        `plain:${span.start_utf16}:${span.end_utf16}:${index}`,
        spoilerText,
        span.reason,
        spoilerState
      )
    );
    cursor = span.end_utf16;
  }

  if (cursor < text.length) {
    nodes.push(
      <Fragment key={`text:${cursor}`}>
        {renderPlainTextSegment(
          text,
          cursor,
          text.length,
          sortedLinks,
          highlightRanges,
          profileUsers,
          onOpenMatrixTarget,
          mentionedUserIds
        )}
      </Fragment>
    );
  }
  return nodes;
}

function renderPlainTextSegment(
  text: string,
  segStart: number,
  segEnd: number,
  sortedLinks: TimelineLinkRange[],
  highlightRanges: TextRange[],
  profileUsers: Record<string, UserProfile>,
  onOpenMatrixTarget: OpenMatrixTargetHandler | undefined,
  mentionedUserIds: ReadonlySet<string>
): ReactNode {
  const nodes: ReactNode[] = [];
  let cursor = segStart;
  for (const range of sortedLinks) {
    if (range.end_utf16 <= cursor || range.start_utf16 >= segEnd) {
      continue;
    }
    const linkStart = Math.max(cursor, range.start_utf16);
    if (linkStart > cursor) {
      nodes.push(
        <Fragment key={`text:${cursor}`}>
          {renderTimelineMessageText(
            text.slice(cursor, linkStart),
            highlightRanges,
            profileUsers,
            cursor,
            mentionedUserIds
          )}
        </Fragment>
      );
    }
    const linkEnd = Math.min(segEnd, range.end_utf16);
    const href = toExternalHttpUrl(range.url);
    const linkContent = renderTimelineMessageText(
      text.slice(linkStart, linkEnd),
      highlightRanges,
      profileUsers,
      linkStart,
      mentionedUserIds
    );
    nodes.push(
      href ? (
        <a
          key={`link:${range.start_utf16}`}
          href={href}
          rel="noopener noreferrer"
          target="_blank"
          onClick={(event) => {
            event.preventDefault();
            activateTimelineLink(href, onOpenMatrixTarget);
          }}
        >
          {linkContent}
        </a>
      ) : (
        <Fragment key={`link:${range.start_utf16}`}>{linkContent}</Fragment>
      )
    );
    cursor = linkEnd;
  }

  if (cursor < segEnd) {
    nodes.push(
      <Fragment key={`text:${cursor}`}>
        {renderTimelineMessageText(
          text.slice(cursor, segEnd),
          highlightRanges,
          profileUsers,
          cursor,
          mentionedUserIds
        )}
      </Fragment>
    );
  }
  return nodes;
}

function normalizeSpoilerSpans(
  spoilerSpans: TimelineItem["spoiler_spans"] | undefined,
  textLength: number
) {
  let cursor = 0;
  return [...(spoilerSpans ?? [])]
    .sort((a, b) => a.start_utf16 - b.start_utf16 || a.end_utf16 - b.end_utf16)
    .flatMap((span) => {
      const start = Math.max(cursor, Math.min(span.start_utf16, textLength));
      const end = Math.max(start, Math.min(span.end_utf16, textLength));
      cursor = end;
      return start < end ? [{ ...span, start_utf16: start, end_utf16: end }] : [];
    });
}

function renderTimelineMessageLine(
  line: string,
  highlightRanges: TextRange[],
  mentionTokens: TimelineMentionToken[],
  baseOffset: number
): ReactNode {
  if (mentionTokens.length === 0) {
    return renderRustHighlights(line, highlightRanges, baseOffset);
  }

  const nodes: ReactNode[] = [];
  let cursor = 0;
  while (cursor < line.length) {
    const next = findNextMentionToken(line, cursor, mentionTokens);
    if (!next) {
      nodes.push(
        <Fragment key={`text:${cursor}`}>
          {renderRustHighlights(line.slice(cursor), highlightRanges, baseOffset + cursor)}
        </Fragment>
      );
      break;
    }
    if (next.start > cursor) {
      nodes.push(
        <Fragment key={`text:${cursor}`}>
          {renderRustHighlights(
            line.slice(cursor, next.start),
            highlightRanges,
            baseOffset + cursor
          )}
        </Fragment>
      );
    }
    const token = line.slice(next.start, next.end);
    nodes.push(
      <span
        className="message-mention-pill"
        data-mention-user-id={next.userId}
        dir="auto"
        key={`${next.userId}:${next.start}`}
      >
        {renderRustHighlights(token, highlightRanges, baseOffset + next.start)}
      </span>
    );
    cursor = next.end;
  }

  return nodes.length > 0
    ? nodes
    : renderRustHighlights(line, highlightRanges, baseOffset);
}

function renderRustHighlights(
  text: string,
  ranges: TextRange[],
  baseOffset: number
): ReactNode {
  const endOffset = baseOffset + text.length;
  const relevant = ranges
    .filter((range) => range.end_utf16 > baseOffset && range.start_utf16 < endOffset)
    .map((range) => ({
      start: Math.max(0, range.start_utf16 - baseOffset),
      end: Math.min(text.length, range.end_utf16 - baseOffset)
    }))
    .filter((range) => range.start < range.end)
    .sort((left, right) => left.start - right.start || left.end - right.end);
  if (relevant.length === 0) return text;

  const nodes: ReactNode[] = [];
  let cursor = 0;
  for (const [index, range] of relevant.entries()) {
    const start = Math.max(cursor, range.start);
    if (start >= range.end) continue;
    if (start > cursor) nodes.push(text.slice(cursor, start));
    nodes.push(<mark key={`${baseOffset + start}:${index}`}>{text.slice(start, range.end)}</mark>);
    cursor = range.end;
  }
  if (cursor < text.length) nodes.push(text.slice(cursor));
  return nodes;
}

type FormattedNode =
  | { kind: "text"; value: string }
  | {
      kind: "element";
      tagName: string;
      attrs: Record<string, string>;
      children: FormattedNode[];
    };

const FORMATTED_TAGS = new Set([
  "a",
  "b",
  "blockquote",
  "br",
  "code",
  "del",
  "div",
  "em",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "i",
  "li",
  "ol",
  "p",
  "pre",
  "s",
  "span",
  "strong",
  "ul"
]);

const VOID_FORMATTED_TAGS = new Set(["br"]);

export function renderFormattedBody(
  formatted: NonNullable<TimelineItem["formatted"]>,
  linkRanges: TimelineLinkRange[],
  codeBlockWrap: boolean,
  onCopyText: TimelineRowActionHandlers["onCopyText"],
  highlightRanges: TextRange[],
  spoilerState: SpoilerRevealState,
  onOpenMatrixTarget: OpenMatrixTargetHandler | undefined,
  mentionedUserIds: ReadonlySet<string> = NO_MENTIONED_USER_IDS
): ReactNode {
  const parsed =
    linkRanges.length > 0 && !formatted.html.includes("<a")
      ? linkifyFormattedNodes(parseFormattedHtml(formatted.html), linkRanges)
      : parseFormattedHtml(formatted.html);
  const nodes = markMentionAnchors(parsed, mentionedUserIds);
  const codeBlockIndexRef = { current: 0 };
  const textOffsetRef = { current: 0 };
  const projectedHighlightRanges = formattedTextOffsetsProject(nodes, formatted.plain_text)
    ? highlightRanges
    : [];
  return renderFormattedNodes(
    nodes,
    formatted,
    codeBlockWrap,
    codeBlockIndexRef,
    onCopyText,
    projectedHighlightRanges,
    textOffsetRef,
    spoilerState,
    onOpenMatrixTarget
  );
}

function formattedTextOffsetsProject(nodes: FormattedNode[], plainText: string): boolean {
  const cursor = { current: 0 };
  const visit = (candidates: FormattedNode[], parentTagName: string | null): boolean => {
    const rendered =
      parentTagName === "ul" || parentTagName === "ol"
        ? candidates.filter((node) => node.kind !== "text" || node.value.trim().length > 0)
        : candidates;
    for (const node of rendered) {
      if (node.kind === "text") {
        const projectedOffset = plainText.indexOf(node.value, cursor.current);
        if (projectedOffset < 0) return false;
        cursor.current = projectedOffset + node.value.length;
      } else if (!visit(node.children, node.tagName)) {
        return false;
      }
    }
    return true;
  };
  return visit(nodes, null);
}

/**
 * Issue #874: tag the `<a>` anchors whose `matrix.to` target is a user this
 * message's `m.mentions` named. The anchor renderer then draws the pill the
 * composer uses, instead of a generic link indistinguishable from a web URL.
 */
function markMentionAnchors(
  nodes: FormattedNode[],
  mentionedUserIds: ReadonlySet<string>
): FormattedNode[] {
  if (mentionedUserIds.size === 0) {
    return nodes;
  }
  const visit = (candidates: FormattedNode[]): FormattedNode[] =>
    candidates.map((node) => {
      if (node.kind !== "element") {
        return node;
      }
      const children = visit(node.children);
      const target = node.tagName === "a" ? matrixPermalinkUser(node.attrs.href ?? "") : null;
      if (!target || !mentionedUserIds.has(target)) {
        return children === node.children ? node : { ...node, children };
      }
      return {
        ...node,
        attrs: { ...node.attrs, "data-mention-user-id": target },
        children
      };
    });
  return visit(nodes);
}

function matrixPermalinkUser(href: string): string | null {
  const target = parseMatrixPermalink(href);
  return target?.kind === "user" ? target.userId : null;
}

function parseFormattedHtml(html: string): FormattedNode[] {
  const root: Extract<FormattedNode, { kind: "element" }> = {
    kind: "element",
    tagName: "fragment",
    attrs: {},
    children: []
  };
  const stack: Array<Extract<FormattedNode, { kind: "element" }>> = [root];
  // Rust owns Matrix HTML safety and emits normalized sanitized HTML. This
  // tokenizer is only a renderer adapter for that DTO, not a sanitizer.
  const tokenPattern = /<!--[\s\S]*?-->|<\/?[^>]+>|[^<]+/g;
  for (const match of html.matchAll(tokenPattern)) {
    const token = match[0];
    if (token.startsWith("<!--")) {
      continue;
    }
    if (token.startsWith("</")) {
      const closeName = token.slice(2, -1).trim().toLowerCase();
      if (!closeName) {
        continue;
      }
      for (let index = stack.length - 1; index >= 0; index -= 1) {
        if (stack[index].tagName === closeName) {
          stack.length = index;
          break;
        }
      }
      continue;
    }
    if (token.startsWith("<")) {
      const parsed = parseFormattedStartTag(token);
      if (!parsed) {
        continue;
      }
      const node: FormattedNode = {
        kind: "element",
        tagName: parsed.tagName,
        attrs: parsed.attrs,
        children: []
      };
      stack[stack.length - 1].children.push(node);
      if (!parsed.selfClosing && !VOID_FORMATTED_TAGS.has(parsed.tagName)) {
        stack.push(node);
      }
      continue;
    }
    stack[stack.length - 1].children.push({ kind: "text", value: decodeHtmlEntities(token) });
  }
  return root.children;
}

function linkifyFormattedNodes(
  nodes: FormattedNode[],
  linkRanges: TimelineLinkRange[]
): FormattedNode[] {
  const sortedRanges = [...linkRanges].sort((left, right) => {
    if (left.start_utf16 !== right.start_utf16) {
      return left.start_utf16 - right.start_utf16;
    }
    return left.end_utf16 - right.end_utf16;
  });
  const cursor = { utf16: 0 };
  return linkifyFormattedNodeList(nodes, sortedRanges, cursor);
}

function linkifyFormattedNodeList(
  nodes: FormattedNode[],
  linkRanges: TimelineLinkRange[],
  cursor: { utf16: number }
): FormattedNode[] {
  return nodes.flatMap((node) => linkifyFormattedNode(node, linkRanges, cursor));
}

function linkifyFormattedNode(
  node: FormattedNode,
  linkRanges: TimelineLinkRange[],
  cursor: { utf16: number }
): FormattedNode[] {
  if (node.kind === "text") {
    const textStart = cursor.utf16;
    cursor.utf16 += node.value.length;
    return linkifyFormattedTextNode(node.value, textStart, linkRanges);
  }

  return [
    {
      ...node,
      children: linkifyFormattedNodeList(node.children, linkRanges, cursor)
    }
  ];
}

function linkifyFormattedTextNode(
  value: string,
  textStartUtf16: number,
  linkRanges: TimelineLinkRange[]
): FormattedNode[] {
  const textEndUtf16 = textStartUtf16 + value.length;
  const rangesInText = linkRanges.filter(
    (range) =>
      range.start_utf16 >= textStartUtf16 &&
      range.end_utf16 <= textEndUtf16 &&
      range.start_utf16 < range.end_utf16
  );
  if (rangesInText.length === 0) {
    return [{ kind: "text", value }];
  }

  const nodes: FormattedNode[] = [];
  let cursor = 0;
  for (const range of rangesInText) {
    const start = range.start_utf16 - textStartUtf16;
    const end = range.end_utf16 - textStartUtf16;
    if (start < cursor) {
      continue;
    }
    if (start > cursor) {
      nodes.push({ kind: "text", value: value.slice(cursor, start) });
    }
    nodes.push({
      kind: "element",
      tagName: "a",
      attrs: { href: range.url },
      children: [{ kind: "text", value: value.slice(start, end) }]
    });
    cursor = end;
  }
  if (cursor < value.length) {
    nodes.push({ kind: "text", value: value.slice(cursor) });
  }
  return nodes;
}

function parseFormattedStartTag(
  token: string
): { tagName: string; attrs: Record<string, string>; selfClosing: boolean } | null {
  const inner = token.slice(1, -1).trim();
  const selfClosing = inner.endsWith("/");
  const withoutSlash = selfClosing ? inner.slice(0, -1).trim() : inner;
  const tagMatch = withoutSlash.match(/^([a-z0-9-]+)/i);
  if (!tagMatch) {
    return null;
  }
  const tagName = tagMatch[1].toLowerCase();
  const attrs: Record<string, string> = {};
  if (FORMATTED_TAGS.has(tagName)) {
    const attrPattern = /([^\s=/>]+)(?:\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+)))?/g;
    for (const match of withoutSlash.slice(tagMatch[0].length).matchAll(attrPattern)) {
      const name = match[1].toLowerCase();
      const value = decodeHtmlEntities(match[3] ?? match[4] ?? match[5] ?? "");
      attrs[name] = value;
    }
  }
  return { tagName, attrs, selfClosing };
}

function renderFormattedNodes(
  nodes: FormattedNode[],
  formatted: NonNullable<TimelineItem["formatted"]>,
  codeBlockWrap: boolean,
  codeBlockIndexRef: { current: number },
  onCopyText: TimelineRowActionHandlers["onCopyText"],
  highlightRanges: TextRange[],
  textOffsetRef: { current: number },
  spoilerState: SpoilerRevealState,
  onOpenMatrixTarget: OpenMatrixTargetHandler | undefined,
  keyPrefix = "",
  parentTagName: string | null = null
): ReactNode {
  const renderedNodes =
    parentTagName === "ul" || parentTagName === "ol"
      ? nodes.filter((node) => node.kind !== "text" || node.value.trim().length > 0)
      : nodes;
  return renderedNodes.map((node, index) =>
    renderFormattedNode(
      node,
      keyPrefix ? `${keyPrefix}.${index}` : `${index}`,
      formatted,
      codeBlockWrap,
      codeBlockIndexRef,
      onCopyText,
      highlightRanges,
      textOffsetRef,
      spoilerState,
      onOpenMatrixTarget
    )
  );
}

function renderFormattedNode(
  node: FormattedNode,
  key: string,
  formatted: NonNullable<TimelineItem["formatted"]>,
  codeBlockWrap: boolean,
  codeBlockIndexRef: { current: number },
  onCopyText: TimelineRowActionHandlers["onCopyText"],
  highlightRanges: TextRange[],
  textOffsetRef: { current: number },
  spoilerState: SpoilerRevealState,
  onOpenMatrixTarget: OpenMatrixTargetHandler | undefined
): ReactNode {
  if (node.kind === "text") {
    const projectedOffset = formatted.plain_text.indexOf(node.value, textOffsetRef.current);
    const baseOffset = projectedOffset >= 0 ? projectedOffset : textOffsetRef.current;
    textOffsetRef.current = baseOffset + node.value.length;
    return (
      <Fragment key={key}>{renderRustHighlights(node.value, highlightRanges, baseOffset)}</Fragment>
    );
  }
  const children = renderFormattedNodes(
    node.children,
    formatted,
    codeBlockWrap,
    codeBlockIndexRef,
    onCopyText,
    highlightRanges,
    textOffsetRef,
    spoilerState,
    onOpenMatrixTarget,
    key,
    node.tagName
  );
  const renderer = formattedTagRenderers[node.tagName as keyof typeof formattedTagRenderers];
  if (!renderer) {
    return <Fragment key={key}>{children}</Fragment>;
  }
  return renderer(
    node,
    key,
    children,
    formatted,
    codeBlockWrap,
    codeBlockIndexRef,
    onCopyText,
    spoilerState,
    onOpenMatrixTarget
  );
}

type SpoilerRevealState = {
  revealed: ReadonlySet<string>;
  reveal: (spoilerKey: string) => void;
};

function renderSpoiler(
  key: string,
  children: ReactNode,
  reason: string | null | undefined,
  spoilerState: SpoilerRevealState
): ReactNode {
  const isRevealed = spoilerState.revealed.has(key);
  const normalizedReason = reason?.trim() || null;
  return (
    <button
      key={key}
      className="message-spoiler"
      type="button"
      data-revealed={isRevealed ? "true" : "false"}
      data-spoiler-reason={normalizedReason ?? undefined}
      aria-label={t("timeline.revealSpoiler")}
      onClick={() => spoilerState.reveal(key)}
    >
      {isRevealed ? children : <span aria-hidden="true">{t("timeline.spoiler")}</span>}
    </button>
  );
}

const MAX_MATH_SOURCE_LENGTH = 1024;
const KATEX_OPTIONS = {
  strict: false,
  throwOnError: false,
  trust: false,
  maxExpand: 1000,
  maxSize: 20
} as const;

function renderMathFormula(
  key: string,
  latex: string | undefined,
  children: ReactNode,
  displayMode: boolean
): ReactNode {
  const source = latex?.trim() ?? "";
  const Tag = displayMode ? "div" : "span";
  if (!source || source.length > MAX_MATH_SOURCE_LENGTH) {
    return (
      <Tag key={key} className={`message-math${displayMode ? " is-block" : ""}`}>
        {children}
      </Tag>
    );
  }
  try {
    const html = katex.renderToString(source, {
      displayMode,
      ...KATEX_OPTIONS
    });
    return (
      <Tag
        key={key}
        className={`message-math${displayMode ? " is-block" : ""}`}
        data-mx-maths={source}
        dangerouslySetInnerHTML={{ __html: html }}
      />
    );
  } catch {
    return (
      <Tag key={key} className={`message-math${displayMode ? " is-block" : ""}`}>
        {children}
      </Tag>
    );
  }
}

type FormattedTagRenderer = (
  node: Extract<FormattedNode, { kind: "element" }>,
  key: string,
  children: ReactNode,
  formatted: NonNullable<TimelineItem["formatted"]>,
  codeBlockWrap: boolean,
  codeBlockIndexRef: { current: number },
  onCopyText: TimelineRowActionHandlers["onCopyText"],
  spoilerState: SpoilerRevealState,
  onOpenMatrixTarget?: OpenMatrixTargetHandler
) => ReactNode;

const formattedTagRenderers: Record<string, FormattedTagRenderer> = {
  a(
    node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"],
    _spoilerState: SpoilerRevealState,
    onOpenMatrixTarget?: OpenMatrixTargetHandler
  ) {
    const href = toExternalHttpUrl(node.attrs.href?.trim());
    if (!href) {
      return <Fragment key={key}>{children}</Fragment>;
    }
    // Issue #874: a real mention reads as the composer's pill, not as a link
    // that looks like any other URL.
    const mentionUserId = node.attrs["data-mention-user-id"];
    return (
      <a
        key={key}
        href={href}
        rel="noopener noreferrer"
        target="_blank"
        className={mentionUserId ? "message-mention-pill" : undefined}
        data-mention-user-id={mentionUserId}
        onClick={(event) => {
          event.preventDefault();
          activateTimelineLink(href, onOpenMatrixTarget);
        }}
      >
        {children}
      </a>
    );
  },
  b(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <strong key={key}>{children}</strong>;
  },
  blockquote(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <blockquote key={key}>{children}</blockquote>;
  },
  br(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    _children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <br key={key} />;
  },
  code(
    node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    const className = node.attrs.class?.trim();
    return (
      <code key={key} className={className || undefined}>
        {children}
      </code>
    );
  },
  del(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <del key={key}>{children}</del>;
  },
  div(
    node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    const math = node.attrs["data-mx-maths"];
    if (math !== undefined) {
      return renderMathFormula(key, math, children, true);
    }
    return <div key={key}>{children}</div>;
  },
  em(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <em key={key}>{children}</em>;
  },
  h1(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h1 key={key}>{children}</h1>;
  },
  h2(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h2 key={key}>{children}</h2>;
  },
  h3(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h3 key={key}>{children}</h3>;
  },
  h4(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h4 key={key}>{children}</h4>;
  },
  h5(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h5 key={key}>{children}</h5>;
  },
  h6(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h6 key={key}>{children}</h6>;
  },
  i(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <em key={key}>{children}</em>;
  },
  li(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <li key={key}>{children}</li>;
  },
  ol(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <ol key={key}>{children}</ol>;
  },
  p(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <p key={key}>{children}</p>;
  },
  pre(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    _children: ReactNode,
    formatted: NonNullable<TimelineItem["formatted"]>,
    codeBlockWrap: boolean,
    codeBlockIndexRef: { current: number },
    onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    const codeBlock = formatted.code_blocks[codeBlockIndexRef.current];
    codeBlockIndexRef.current += 1;
    if (!codeBlock) {
      return <pre key={key} />;
    }
    const languageClass = codeBlock.language ? `language-${codeBlock.language}` : null;
    return (
      <div key={key} className="message-code-block">
        <div className="message-code-block-actions">
          <button
            className="message-code-block-copy"
            type="button"
            aria-label={t("timeline.copyCode")}
            onClick={() => onCopyText(codeBlock.body)}
          >
            <Copy size={13} aria-hidden="true" />
            <span>{t("timeline.copyCode")}</span>
          </button>
        </div>
        <pre className="message-code-block-pre" data-code-block-wrap={codeBlockWrap ? "true" : "false"}>
          <code className={languageClass || undefined}>{codeBlock.body}</code>
        </pre>
      </div>
    );
  },
  s(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <del key={key}>{children}</del>;
  },
  span(
    node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"],
    spoilerState: SpoilerRevealState
  ) {
    const className = node.attrs.class?.trim();
    const spoiler = node.attrs["data-mx-spoiler"];
    const math = node.attrs["data-mx-maths"];
    const color = node.attrs["data-mx-color"];
    if (math !== undefined) {
      return renderMathFormula(key, math, children, false);
    }
    if (spoiler !== undefined) {
      return renderSpoiler(`formatted:${key}`, children, spoiler, spoilerState);
    }
    return (
      <span
        key={key}
        className={className || undefined}
        data-mx-color={color || undefined}
        data-mx-spoiler={spoiler ?? undefined}
      >
        {children}
      </span>
    );
  },
  strong(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <strong key={key}>{children}</strong>;
  },
  ul(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <ul key={key}>{children}</ul>;
  }
} as const;

function decodeHtmlEntities(text: string): string {
  return text.replace(/&(#x?[0-9a-fA-F]+|[a-zA-Z]+);/g, (match, entity: string) => {
    if (entity.startsWith("#x") || entity.startsWith("#X")) {
      const codePoint = Number.parseInt(entity.slice(2), 16);
      return isValidHtmlCodePoint(codePoint) ? String.fromCodePoint(codePoint) : match;
    }
    if (entity.startsWith("#")) {
      const codePoint = Number.parseInt(entity.slice(1), 10);
      return isValidHtmlCodePoint(codePoint) ? String.fromCodePoint(codePoint) : match;
    }
    switch (entity) {
      case "amp":
        return "&";
      case "lt":
        return "<";
      case "gt":
        return ">";
      case "quot":
        return '"';
      case "apos":
      case "nbsp":
        return entity === "nbsp" ? " " : "'";
      default:
        return match;
    }
  });
}

function isValidHtmlCodePoint(codePoint: number): boolean {
  return Number.isInteger(codePoint) && codePoint >= 0 && codePoint <= 0x10ffff;
}

function findNextMentionToken(
  line: string,
  start: number,
  mentionTokens: TimelineMentionToken[]
): { start: number; end: number; userId: string } | null {
  for (let index = start; index < line.length; index += 1) {
    for (const mention of mentionTokens) {
      const end = index + mention.token.length;
      if (
        line.startsWith(mention.token, index) &&
        hasMentionTokenBoundary(line, index, end)
      ) {
        return { start: index, end, userId: mention.userId };
      }
    }
  }
  return null;
}

function timelineMentionTokens(
  profileUsers: Record<string, UserProfile>,
  mentionedUserIds: ReadonlySet<string>
): TimelineMentionToken[] {
  if (mentionedUserIds.size === 0) {
    return [];
  }
  const tokens = new Map<string, string>();
  for (const profile of Object.values(profileUsers)) {
    if (!mentionedUserIds.has(profile.user_id)) {
      continue;
    }
    const terms = profile.mention_search_terms.length
      ? profile.mention_search_terms
      : [profile.display_label, profile.user_id];
    for (const term of terms) {
      const normalized = term.trim();
      if (normalized) {
        tokens.set(normalized.startsWith("@") ? normalized : `@${normalized}`, profile.user_id);
      }
    }
  }
  return Array.from(tokens, ([token, userId]) => ({ token, userId }))
    .filter((mention) => mention.token.length > 1)
    .sort((a, b) => b.token.length - a.token.length || a.token.localeCompare(b.token));
}

function hasMentionTokenBoundary(line: string, start: number, end: number): boolean {
  return isMentionStartBoundary(line[start - 1]) && isMentionEndBoundary(line[end]);
}

function isMentionStartBoundary(value: string | undefined): boolean {
  return value === undefined || /\s|[([{<]/u.test(value);
}

function isMentionEndBoundary(value: string | undefined): boolean {
  return value === undefined || /\s|[.,!?;:)\]}>]/u.test(value);
}

export async function writeClipboardText(value: string): Promise<void> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }
  if (typeof document === "undefined") {
    return;
  }
  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.insetInlineStart = "-9999px";
  document.body.appendChild(textarea);
  textarea.select();
  document.execCommand("copy");
  document.body.removeChild(textarea);
}
