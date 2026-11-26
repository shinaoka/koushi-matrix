import { createContext, useContext } from "react";

import type { TimelineItem } from "../../domain/coreEvents";

/** A rendered Markdown attachment handed to the right panel (#2097). */
export interface MarkdownDocument {
  filename: string;
  formatted: NonNullable<TimelineItem["formatted"]>;
}

/**
 * Where a file card opens its document. The shell provides it and shows the
 * document in the right panel; without a provider (a standalone timeline) the
 * card has nowhere to open it and keeps the plain card.
 */
export const MarkdownDocumentOpenerContext = createContext<
  ((document: MarkdownDocument) => void) | null
>(null);

export function useMarkdownDocumentOpener(): ((document: MarkdownDocument) => void) | null {
  return useContext(MarkdownDocumentOpenerContext);
}
