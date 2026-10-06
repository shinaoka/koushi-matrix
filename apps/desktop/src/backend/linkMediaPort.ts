/**
 * Facts the save-name policy needs beyond the attachment's own filename (#1135).
 * The renderer resolves them where the platform facts live: the media kind from
 * the item, and the event's local offset from `Date`.
 */
export type MediaSaveNameFacts = {
  kind: "image" | "file";
  timestampMs: number | null;
};

export interface LinkMediaPort {
  openHttpUrl(url: string): Promise<void>;
  mediaSourceUrl(sourceUrl: string): string;
  renderableThumbnailSourceUrl(sourceRef: string): string | null;
  saveMediaFile(
    sourceUrl: string,
    filename: string,
    saveName: MediaSaveNameFacts | null,
    accountTabId?: string
  ): Promise<void>;
}
