/**
 * Typed `matrix.to` permalink parsing.
 *
 * Matrix permalinks are a local navigation target, not an external web link:
 * clicking one should open Koushi's join preview rather than a browser tab. A
 * dedicated parser (rather than a loose URL regex) keeps every entry point —
 * link click, paste into discovery, and any future `/join` command — on one
 * definition of what a target is.
 *
 * Whether a room is a space is not encoded in the URL; that is discovered from
 * the resolved metadata, so this parser never guesses it.
 */

/** A parsed permalink target. */
export type MatrixPermalinkTarget =
  | {
      kind: "room";
      /** `#alias:server` or `!id:server`, already percent-decoded. */
      roomIdOrAlias: string;
      /** Servers to try when the room id is not resolvable locally. */
      viaServers: string[];
      /** Present for event permalinks. */
      eventId?: string;
    }
  | { kind: "user"; userId: string };

const MATRIX_TO_HOSTS = new Set(["matrix.to", "www.matrix.to"]);

/** Matrix entity sigils this parser understands. */
const ROOM_SIGILS = ["#", "!"];
const USER_SIGIL = "@";

/**
 * Parse a `matrix.to` URL into a typed target, or `null` when it is not a
 * Matrix permalink this app can act on.
 *
 * Accepts the entity in the fragment, percent-encoded or not, an optional
 * event id segment, and repeated `via` parameters. Anything else — a different
 * host, a bare fragment, an unknown sigil, an empty server part — is not a
 * target and stays an ordinary link.
 */
export function parseMatrixPermalink(rawUrl: string): MatrixPermalinkTarget | null {
  const trimmed = rawUrl.trim();
  if (trimmed.length === 0) {
    return null;
  }
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") {
    return null;
  }
  if (!MATRIX_TO_HOSTS.has(url.hostname.toLowerCase())) {
    return null;
  }
  // The entity lives in the fragment: `#/<entity>[/<eventId>][?via=...]`.
  const fragment = url.hash.startsWith("#") ? url.hash.slice(1) : url.hash;
  if (!fragment.startsWith("/")) {
    return null;
  }
  const [pathPart, queryPart] = splitFragment(fragment.slice(1));
  const segments = pathPart
    .split("/")
    .filter((segment) => segment.length > 0)
    .map(decodeSegment);
  if (segments.length === 0 || segments.some((segment) => segment === null)) {
    return null;
  }
  const [entity, maybeEventId] = segments as string[];
  if (!isWellFormedEntity(entity)) {
    return null;
  }

  if (entity.startsWith(USER_SIGIL)) {
    // A user permalink carries no room, so a trailing segment is meaningless.
    return maybeEventId === undefined ? { kind: "user", userId: entity } : null;
  }

  const viaServers = readViaServers(queryPart);
  if (maybeEventId === undefined) {
    return { kind: "room", roomIdOrAlias: entity, viaServers };
  }
  if (!maybeEventId.startsWith("$") || maybeEventId.length < 2) {
    return null;
  }
  return { kind: "room", roomIdOrAlias: entity, viaServers, eventId: maybeEventId };
}

/**
 * Normalize free-text input to the entity a permalink points at.
 *
 * Pasting a `matrix.to` URL into a search or join field should behave exactly
 * like typing the alias, so the discovery surface never needs its own parser.
 * Non-permalink input is returned trimmed and unchanged.
 */
export function normalizeMatrixTargetInput(rawInput: string): string {
  const trimmed = rawInput.trim();
  const target = parseMatrixPermalink(trimmed);
  if (!target) {
    return trimmed;
  }
  return target.kind === "room" ? target.roomIdOrAlias : target.userId;
}

/** Server part of a Matrix id/alias, used as a `via` hint for alias joins. */
export function serverNameFromMatrixId(idOrAlias: string): string | null {
  const separator = idOrAlias.indexOf(":");
  if (separator <= 0 || separator === idOrAlias.length - 1) {
    return null;
  }
  return idOrAlias.slice(separator + 1);
}

function splitFragment(fragment: string): [string, string] {
  const queryStart = fragment.indexOf("?");
  return queryStart === -1
    ? [fragment, ""]
    : [fragment.slice(0, queryStart), fragment.slice(queryStart + 1)];
}

function decodeSegment(segment: string): string | null {
  try {
    return decodeURIComponent(segment);
  } catch {
    // A malformed escape is not a target; treating it as literal text would
    // silently join the wrong room.
    return null;
  }
}

function isWellFormedEntity(entity: string): boolean {
  const sigil = entity.slice(0, 1);
  if (![...ROOM_SIGILS, USER_SIGIL].includes(sigil)) {
    return false;
  }
  const separator = entity.indexOf(":");
  // `#localpart:server` — both parts must be non-empty.
  return separator > 1 && separator < entity.length - 1;
}

function readViaServers(queryPart: string): string[] {
  if (queryPart.length === 0) {
    return [];
  }
  const params = new URLSearchParams(queryPart);
  const seen = new Set<string>();
  for (const value of params.getAll("via")) {
    const server = value.trim();
    if (server.length > 0) {
      seen.add(server);
    }
  }
  return [...seen];
}
