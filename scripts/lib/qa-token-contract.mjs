// Shared QA token / private-data assertions for real and headless QA wrappers.
//
// QA scripts must assert scenario-specific success tokens (not just process exit
// code), and real-account/real-homeserver QA output must never carry private
// Matrix identifiers. These helpers enforce both contracts directly in the Node
// wrappers so a passing exit code can never mask a missing token or a leaked id.

/**
 * Parse the set of `name=value` success tokens from QA output. Only the closed
 * vocabulary of status values is accepted so prose can never be mistaken for a
 * token.
 */
export function tokensFromOutput(output) {
  return new Set(
    String(output)
      .split(/\s+/)
      .filter((token) =>
        /^[a-z0-9_]+=(ok|running|created|not_found|completed|partial)$/.test(token)
      )
  );
}

/**
 * Throw unless every required token is present in the output. Asserts the
 * scenario actually reached each documented checkpoint.
 */
export function assertRequiredTokens(output, requiredTokens, label) {
  const tokens = tokensFromOutput(output);
  const missing = requiredTokens.filter((token) => !tokens.has(token));
  if (missing.length > 0) {
    throw new Error(`${label}: missing required QA tokens: ${missing.join(", ")}`);
  }
}

/**
 * Throw if any Matrix identifier (@user:server, !room:server, $event:server)
 * appears in the output. Real QA must surface only private-data-free tokens.
 */
export function assertNoMatrixIdentifiers(output, label) {
  const text = String(output);
  // Boundary includes `=` so the net also catches ids attached to a token key
  // (e.g. `user_id=@user:server`) — the exact format the old binary leaked — not
  // only whitespace-delimited ids.
  const matrixIdPattern = /(?:^|[\s=])([@!$][A-Za-z0-9._=\-]+:[A-Za-z0-9.\-]+)(?:\s|$)/;
  const match = text.match(matrixIdPattern);
  if (match) {
    throw new Error(`${label}: private Matrix identifier leaked into QA output`);
  }
}

/**
 * Throw if a local filesystem path appears in output that may become an issue
 * artifact. Keep the diagnostic generic so the rejection itself is shareable.
 */
export function assertNoLocalPaths(output, label) {
  const text = String(output);
  const localPathPattern =
    /(?:^|[\s"'(=])((?:\/(?:home|Users|tmp|private\/tmp|var\/folders|workspace|runner|Volumes)\/[^\s"'<>]*)|(?:[A-Za-z]:\\[^\s"'<>]+))/;
  if (localPathPattern.test(text)) {
    throw new Error(`${label}: local filesystem path leaked into QA output`);
  }
}

/**
 * Throw on common raw SDK/runtime diagnostic shapes. Public QA output should
 * contain stable product tokens and coarse failure kinds, not library internals.
 */
export function assertNoRawSdkErrors(output, label) {
  const text = String(output);
  const rawSdkPattern =
    /\b(?:matrix_sdk::|ruma::|reqwest::|hyper::|SdkError|HttpError|ClientApiError|StoreError|ServerError|M_[A-Z0-9_]+)\b/;
  if (rawSdkPattern.test(text)) {
    throw new Error(`${label}: raw SDK diagnostic leaked into QA output`);
  }
}
