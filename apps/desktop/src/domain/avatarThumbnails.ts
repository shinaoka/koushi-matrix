import type { AvatarImage, DesktopSnapshot } from "./types";

/** Renderer demand discovery only; Core owns request dedupe, retries and terminal state. */
export const AVATAR_THUMBNAIL_DOWNLOADS_ENABLED = true;

export function resolvedAvatar(
  itemAvatar: AvatarImage | null | undefined,
  profileAvatar: AvatarImage | null | undefined,
  knownAvatar?: AvatarImage | null
): AvatarImage | null {
  const avatar = profileAvatar && itemAvatar && profileAvatar.mxc_uri === itemAvatar.mxc_uri
    ? profileAvatar
    : itemAvatar ?? profileAvatar ?? null;
  // Reuse a Rust-owned ready thumbnail only for the exact same media resource.
  // A room-specific sender avatar must keep its own URI and demand lifetime.
  return avatar && knownAvatar?.thumbnail.kind === "ready" && knownAvatar.mxc_uri === avatar.mxc_uri
    ? knownAvatar
    : avatar;
}

export interface AvatarThumbnailRequestPlan {
  requestMxcUris: string[];
  requestedMxcUris: Set<string>;
}

export function requestAvatarThumbnailWithDedupe(
  mxcUri: string,
  snapshotRequestedMxcUris: ReadonlySet<string>,
  visibleRequestedMxcUris: Set<string>,
  request: ((mxcUri: string) => Promise<void>) | undefined
): Promise<void> {
  const normalizedMxcUri = mxcUri.trim();
  if (
    !normalizedMxcUri ||
    !request ||
    snapshotRequestedMxcUris.has(normalizedMxcUri) ||
    visibleRequestedMxcUris.has(normalizedMxcUri)
  ) {
    return Promise.resolve();
  }

  visibleRequestedMxcUris.add(normalizedMxcUri);
  try {
    return Promise.resolve(request(normalizedMxcUri)).catch(() => {
      // Admission/transport failed before Core could own the request.
      visibleRequestedMxcUris.delete(normalizedMxcUri);
    });
  } catch {
    visibleRequestedMxcUris.delete(normalizedMxcUri);
    return Promise.resolve();
  }
}

export function planSnapshotAvatarThumbnailRequests(
  snapshot: DesktopSnapshot,
  previousRequestedMxcUris: ReadonlySet<string>
): AvatarThumbnailRequestPlan {
  const candidates = collectNotRequestedAvatarMxcUris(snapshot);
  const requestedMxcUris = new Set(
    [...previousRequestedMxcUris].filter((mxcUri) => candidates.has(mxcUri))
  );
  const requestMxcUris = [...candidates].filter((mxcUri) => !requestedMxcUris.has(mxcUri));
  requestMxcUris.forEach((mxcUri) => requestedMxcUris.add(mxcUri));
  return { requestMxcUris, requestedMxcUris };
}

function collectNotRequestedAvatarMxcUris(snapshot: DesktopSnapshot): Set<string> {
  const candidates = new Set<string>();
  // Room, Space, and invite rows report demand from their rendered viewport;
  // keeping them here would eagerly request every account-wide icon.
  const avatars: Array<AvatarImage | null> = [snapshot.state.domain.profile.own.avatar];

  for (const avatar of avatars) {
    if (avatar?.thumbnail.kind === "notRequested") candidates.add(avatar.mxc_uri);
  }
  return candidates;
}
