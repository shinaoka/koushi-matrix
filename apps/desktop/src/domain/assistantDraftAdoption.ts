import type { AssistantDraftMarker, ComposerDocument, ComposerDraftRevision } from "./types";
import { compareComposerDraftRevisions } from "./composerDraftRevision";
import { plainBodyFromDocument } from "./composerDocument";

/**
 * #1644: which document an open composer shows when Rust reports an
 * assistant-staged draft while the renderer may hold a local overlay.
 *
 * - `overlay`: the user's local text is newer than the staged draft (they
 *   edited it), or there is no staged draft.
 * - `snapshot`: no overlay; the Rust document is shown as usual.
 * - `adopt`: the overlay is empty and older than the staged draft, so the
 *   staged draft replaces it and the overlay is retired.
 * - `user_wins`: the user typed in the persistence debounce window just as the
 *   draft landed. Their text stays, is re-persisted above the staged revision,
 *   and the assistant marker is dismissed.
 */
export type AssistantDraftOverlayDecision = "overlay" | "snapshot" | "adopt" | "user_wins";

export interface ComposerOverlayView {
  document: ComposerDocument;
  revision: ComposerDraftRevision | null;
}

export function assistantDraftOverlayDecision(
  overlay: ComposerOverlayView | null,
  marker: AssistantDraftMarker | null | undefined
): AssistantDraftOverlayDecision {
  if (!overlay) return "snapshot";
  if (!marker) return "overlay";
  // An edit made after the draft was observed is minted above its revision.
  // An equal revision was minted before: Rust refuses that write, so it is
  // the debounce race, not an edit.
  if (
    overlay.revision !== null &&
    compareComposerDraftRevisions(marker.revision, overlay.revision) < 0
  ) {
    return "overlay";
  }
  return plainBodyFromDocument(overlay.document).length === 0 ? "adopt" : "user_wins";
}

/** Whether the composer renders the overlay rather than the Rust document. */
export function overlayIsShown(decision: AssistantDraftOverlayDecision): boolean {
  return decision === "overlay" || decision === "user_wins";
}
