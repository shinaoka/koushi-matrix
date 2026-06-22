import type { AvatarImage, AvatarThumbnailState, DesktopSnapshot } from "./types";

/**
 * Temporary #116 perf gate: default OFF until avatar downloads are re-enabled
 * behind a Rust-owned setting + encrypted cache + bounded worker pool.
 * Set to true to restore the pre-#116 firing behaviour (for tests or future
 * re-enablement).
 */
export const AVATAR_THUMBNAIL_DOWNLOADS_ENABLED = false;

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
  const avatars: Array<AvatarImage | null> = [
    snapshot.state.domain.profile.own.avatar,
    ...Object.values(snapshot.state.domain.profile.users).map((profile) => profile.avatar),
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
