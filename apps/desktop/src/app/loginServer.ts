// Element X-style sign-in (#1101): the Matrix ID is the primary input and the
// homeserver is derived from its server name. Core resolves the server name
// through `/.well-known/matrix/client`, so the renderer only picks which name
// to send; it never resolves or validates homeservers itself.

export const DEFAULT_LOGIN_SERVER = "matrix.org";

/** Server name of a full Matrix ID (`@alice:example.org` -> `example.org`). */
export function matrixIdServerName(input: string): string | null {
  const trimmed = input.trim();
  if (!trimmed.startsWith("@")) return null;
  const separator = trimmed.indexOf(":");
  if (separator <= 1) return null;
  const serverName = trimmed.slice(separator + 1).trim();
  return serverName.length > 0 ? serverName : null;
}

/**
 * The server the sign-in form talks to: an explicit override when the user
 * chose one, otherwise the Matrix ID's server name, otherwise the default.
 */
export function effectiveLoginServer(matrixId: string, serverOverride: string | null): string {
  if (serverOverride !== null) return serverOverride.trim();
  return matrixIdServerName(matrixId) ?? DEFAULT_LOGIN_SERVER;
}

/** Host shown in the server summary, without scheme or trailing slash. */
export function loginServerDisplayName(server: string): string {
  return server.trim().replace(/^https?:\/\//i, "").replace(/\/+$/, "");
}
