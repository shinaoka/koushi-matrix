import { resolvedAvatar } from "../../domain/avatarThumbnails";
import type { TimelineItem } from "../../domain/coreEvents";
import type { UserProfile } from "../../domain/types";

export interface TimelineDiagnostics {
  visibleItems: number;
  downloadedItems: number;
  backfill: string;
  avatarMxcItems: number;
  avatarReadyItems: number;
  avatarPendingItems: number;
  avatarFailedItems: number;
  avatarMissingItems: number;
  avatarRenderedImages: number;
  avatarBrokenImages: number;
}

export function timelineAvatarDiagnostics(
  items: readonly TimelineItem[],
  profileUsers: Record<string, UserProfile>
): Omit<
  TimelineDiagnostics,
  "visibleItems" | "downloadedItems" | "backfill" | "avatarRenderedImages" | "avatarBrokenImages"
> {
  const diagnostics = {
    avatarMxcItems: 0,
    avatarReadyItems: 0,
    avatarPendingItems: 0,
    avatarFailedItems: 0,
    avatarMissingItems: 0
  };
  for (const item of items) {
    const profileAvatar = item.sender ? profileUsers[item.sender]?.avatar : null;
    const avatar = resolvedAvatar(item.sender_avatar, profileAvatar);
    if (!avatar) {
      diagnostics.avatarMissingItems += 1;
      continue;
    }
    diagnostics.avatarMxcItems += 1;
    const thumbnail = avatar.thumbnail;
    if (thumbnail.kind === "ready") {
      diagnostics.avatarReadyItems += 1;
    } else if (thumbnail.kind === "failed") {
      diagnostics.avatarFailedItems += 1;
    } else {
      diagnostics.avatarPendingItems += 1;
    }
  }
  return diagnostics;
}

export function timelineRenderedAvatarDiagnostics(container: HTMLElement | null): {
  avatarRenderedImages: number;
  avatarBrokenImages: number;
} {
  if (!container) {
    return { avatarRenderedImages: 0, avatarBrokenImages: 0 };
  }
  const images = Array.from(container.querySelectorAll<HTMLImageElement>(".avatar img"));
  return {
    avatarRenderedImages: images.length,
    avatarBrokenImages: images.filter((image) => image.complete && image.naturalWidth === 0).length
  };
}
