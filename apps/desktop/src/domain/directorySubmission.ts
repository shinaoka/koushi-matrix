/**
 * What submitting the Explore field should do.
 *
 * One field handles both searching and opening a link, because splitting them
 * would force the user to decide which input a pasted string belongs to before
 * they can act, and both inputs would run the same normalization anyway.
 */

import { parseMatrixPermalink } from "./matrixPermalink";

export type DirectorySubmission =
  | { kind: "empty" }
  | { kind: "search"; term: string }
  | { kind: "join"; roomIdOrAlias: string; viaServers: string[] }
  /** A person is not joinable; the caller decides how to surface them. */
  | { kind: "user"; userId: string };

const ROOM_SIGILS = ["#", "!"];

/**
 * Classify free-text Explore input.
 *
 * A permalink or a sigil-qualified id names exactly one room, so it is opened
 * rather than searched for - a directory text search would not even find a room
 * addressed by id. Anything else, including a sigil with no server part, is a
 * search term, because it does not identify a single room.
 */
export function resolveDirectorySubmission(rawInput: string): DirectorySubmission {
  const trimmed = rawInput.trim();
  if (trimmed.length === 0) {
    return { kind: "empty" };
  }

  const permalink = parseMatrixPermalink(trimmed);
  if (permalink) {
    return permalink.kind === "user"
      ? { kind: "user", userId: permalink.userId }
      : {
          kind: "join",
          roomIdOrAlias: permalink.roomIdOrAlias,
          viaServers: permalink.viaServers
        };
  }

  if (namesOneRoom(trimmed)) {
    // Typed directly, so there is no link to carry routing hints.
    return { kind: "join", roomIdOrAlias: trimmed, viaServers: [] };
  }
  return { kind: "search", term: trimmed };
}

function namesOneRoom(input: string): boolean {
  if (!ROOM_SIGILS.includes(input.slice(0, 1))) {
    return false;
  }
  const separator = input.indexOf(":");
  // `#localpart:server` — without both parts this is just text to search for.
  return separator > 1 && separator < input.length - 1;
}
