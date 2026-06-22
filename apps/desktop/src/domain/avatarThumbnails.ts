import type { AvatarImage, AvatarThumbnailState, DesktopSnapshot } from "./types";

/**
 * #116 kill-switch: avatars are re-enabled with member-avatar bulk requests
 * removed (#116 Stage F1a); set to false to disable all avatar downloads.
 */
export const AVATAR_THUMBNAIL_DOWNLOADS_ENABLED = true;

export const MAX_AVATAR_THUMBNAIL_ATTEMPTS = 2;

export interface AvatarThumbnailRequestPlan {
  requestMxcUris: string[];
  requestedMxcUris: Set<string>;
  retryCounts: Map<string, number>;
}

interface AvatarThumbnailRequestCandidate {
  mxcUri: string;
  thumbnail: AvatarThumbnailState;
}

export function avatarThumbnailRequestShouldBeSkipped(
  thumbnail: AvatarThumbnailState
): boolean {
  if (thumbnail.kind === "ready") {
    return true;
  }
  return thumbnail.kind === "failed" && !avatarThumbnailFailureIsRetryable(thumbnail);
}

export function avatarThumbnailFailureIsRetryable(thumbnail: AvatarThumbnailState): boolean {
  return (
    thumbnail.kind === "failed" &&
    (thumbnail.failureKind === "network" || thumbnail.failureKind === "sdk")
  );
}

export function planSnapshotAvatarThumbnailRequests(
  snapshot: DesktopSnapshot,
  previousRequestedMxcUris: ReadonlySet<string>,
  previousRetryCounts: ReadonlyMap<string, number>,
  maxAttempts = MAX_AVATAR_THUMBNAIL_ATTEMPTS
): AvatarThumbnailRequestPlan {
  const candidates = collectSnapshotAvatarThumbnailRequestCandidates(snapshot);
  const requestedMxcUris = new Set(previousRequestedMxcUris);
  const retryCounts = new Map(previousRetryCounts);
  const requestMxcUris: string[] = [];

  for (const mxcUri of previousRequestedMxcUris) {
    const candidate = candidates.get(mxcUri);
    if (!candidate) {
      requestedMxcUris.delete(mxcUri);
      retryCounts.delete(mxcUri);
      continue;
    }
    if (avatarThumbnailFailureIsRetryable(candidate.thumbnail)) {
      requestedMxcUris.delete(mxcUri);
    }
  }

  for (const candidate of candidates.values()) {
    if (requestedMxcUris.has(candidate.mxcUri)) {
      continue;
    }
    const attempts = retryCounts.get(candidate.mxcUri) ?? 0;
    if (attempts >= maxAttempts) {
      continue;
    }
    requestMxcUris.push(candidate.mxcUri);
    requestedMxcUris.add(candidate.mxcUri);
    retryCounts.set(candidate.mxcUri, attempts + 1);
  }

  return { requestMxcUris, requestedMxcUris, retryCounts };
}

function collectSnapshotAvatarThumbnailRequestCandidates(
  snapshot: DesktopSnapshot
): Map<string, AvatarThumbnailRequestCandidate> {
  const candidates = new Map<string, AvatarThumbnailRequestCandidate>();
  const completed = new Set<string>();
  // NOTE: profile.users (member avatar cache) is intentionally excluded here.
  // Eagerly requesting all member avatars caused ~1421 simultaneous downloads on
  // large Room Info panels and froze the app (#116). Visible message-sender
  // avatars are requested per-row by the TimelineView virtualized effect;
  // member-list avatars will be added per-visible-member in Stage F1b.
  const avatars: Array<AvatarImage | null> = [
    snapshot.state.domain.profile.own.avatar,
    ...snapshot.state.domain.rooms.map((room) => room.avatar),
    ...snapshot.state.domain.spaces.map((space) => space.avatar),
    ...snapshot.state.domain.invites.map((invite) => invite.avatar)
  ];

  for (const avatar of avatars) {
    if (!avatar) {
      continue;
    }
    if (avatar.thumbnail.kind === "ready") {
      completed.add(avatar.mxc_uri);
      candidates.delete(avatar.mxc_uri);
      continue;
    }
    if (completed.has(avatar.mxc_uri) || avatarThumbnailRequestShouldBeSkipped(avatar.thumbnail)) {
      continue;
    }
    candidates.set(avatar.mxc_uri, {
      mxcUri: avatar.mxc_uri,
      thumbnail: avatar.thumbnail
    });
  }

  return candidates;
}
