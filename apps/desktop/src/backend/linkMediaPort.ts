export interface LinkMediaPort {
  openHttpUrl(url: string): Promise<void>;
  mediaSourceUrl(sourceUrl: string): string;
  renderableThumbnailSourceUrl(sourceRef: string): string | null;
  saveMediaFile(sourceUrl: string, filename: string): Promise<void>;
}
