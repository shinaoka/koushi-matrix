import type { AvatarThumbnailState, TimelineMediaSource } from "./coreEvents";

export type LinkPreviewState = "pending" | "loading" | "ready" | "failed";

export interface LinkPreviewImage {
  source: TimelineMediaSource;
  width?: number;
  height?: number;
  thumbnail: AvatarThumbnailState;
}

export interface LinkPreview {
  url: string;
  title?: string;
  description?: string;
  image?: LinkPreviewImage;
  state: LinkPreviewState;
}
