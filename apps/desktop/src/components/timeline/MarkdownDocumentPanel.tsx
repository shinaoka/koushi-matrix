import { useEffect, useRef, useState } from "react";

import { t } from "../../i18n/messages";
import { renderMarkdownDocument } from "./TimelineMessageBody";
import type { MarkdownDocument } from "./MarkdownDocumentContext";

interface TocEntry {
  id: string;
  level: number;
  text: string;
}

/**
 * #2097: the whole Markdown document, with a table of contents.
 *
 * The body is the message renderer's output for Core's sanitized DTO; the table
 * of contents is read back from the rendered h1-h3 headings, so it can only list
 * what the reader can see, and a document without headings has none.
 */
export function MarkdownDocumentPanel({
  document: doc,
  onCopyText
}: {
  document: MarkdownDocument;
  onCopyText: (text: string) => void;
}) {
  const bodyRef = useRef<HTMLDivElement>(null);
  const [toc, setToc] = useState<TocEntry[]>([]);
  useEffect(() => {
    const headings = bodyRef.current?.querySelectorAll<HTMLElement>("h1[id], h2[id], h3[id]");
    setToc(
      Array.from(headings ?? []).map((heading) => ({
        id: heading.id,
        level: Number(heading.tagName.slice(1)),
        text: heading.textContent ?? ""
      }))
    );
  }, [doc]);
  const goTo = (id: string) => {
    const target = Array.from(
      bodyRef.current?.querySelectorAll<HTMLElement>("[id]") ?? []
    ).find((element) => element.id === id);
    target?.scrollIntoView?.({ block: "start" });
  };
  return (
    <div className="markdown-document" data-testid="markdown-document">
      {toc.length > 0 ? (
        <nav className="markdown-document-toc" aria-label={t("timeline.mediaDocumentContents")}>
          <ul>
            {toc.map((entry) => (
              <li key={entry.id} data-level={entry.level}>
                <button type="button" onClick={() => goTo(entry.id)}>
                  {entry.text}
                </button>
              </li>
            ))}
          </ul>
        </nav>
      ) : null}
      <div className="markdown-document-body message-body" ref={bodyRef} dir="auto">
        {renderMarkdownDocument(doc.formatted, onCopyText)}
      </div>
    </div>
  );
}
