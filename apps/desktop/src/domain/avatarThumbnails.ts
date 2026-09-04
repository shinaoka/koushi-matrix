import type { AvatarImage, DesktopSnapshot } from "./types";

/** Renderer demand discovery only; Core owns request dedupe, retries and terminal state. */
export const AVATAR_THUMBNAIL_DOWNLOADS_ENABLED = true;

export function resolvedAvatar(
  itemAvatar: AvatarImage | null | undefined,
  profileAvatar: AvatarImage | null | undefined
): AvatarImage | null {
  return profileAvatar && itemAvatar && profileAvatar.mxc_uri === itemAvatar.mxc_uri
    ? profileAvatar
    : itemAvatar ?? profileAvatar ?? null;
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
  // profile.users remains visibility-driven to avoid eager member-list downloads.
  const avatars: Array<AvatarImage | null> = [
    snapshot.state.domain.profile.own.avatar,
    ...snapshot.state.domain.rooms.map((room) => room.avatar),
    ...snapshot.state.domain.spaces.map((space) => space.avatar),
    ...snapshot.state.domain.invites.map((invite) => invite.avatar)
  ];

  for (const avatar of avatars) {
    if (avatar?.thumbnail.kind === "notRequested") candidates.add(avatar.mxc_uri);
  }
  return candidates;
}
