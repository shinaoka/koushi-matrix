# Repository Rules

These rules apply to first-party code, docs, tests, and integration glue in this
repository. Vendored upstream code must keep its original license and copyright
notices; local changes to vendored code must remain easy to upstream or revert.

## Security Rules

- Decrypted E2EE event bodies, attachment filenames, snippets, search queries,
  access tokens, refresh tokens, recovery keys, room keys, local store keys, and
  search index keys are secrets.
- Secrets MUST NOT be logged, sent to telemetry, written to crash reports,
  printed in test output, checked into fixtures, or copied into screenshots.
- Decrypted event bodies and plaintext-derived data MUST NOT be persisted in
  first-party stores outside an encrypted Matrix SDK store or encrypted search
  index.
- Ngram terms, token dictionaries, postings, highlight spans, and attachment
  filename matches are plaintext-derived data. Treat them with the same
  confidentiality as the original message text.
- Persistent local search for E2EE rooms MUST use an encrypted
  `matrix-sdk-search` index. Unencrypted search indexes are forbidden for E2EE
  content.
- The search index MUST NOT be the display source of truth. It may produce
  candidate event IDs only; snippets and highlights must be generated from the
  resolved visible event content loaded from the SDK store or network.
- Search result highlights MUST be exact, second-pass verified spans. Ngram
  candidates without a verified visible span must be dropped or shown only by an
  explicitly non-exact result mode.
- Edits, replacements, and redactions MUST be resolved before indexing or
  returning a result. An edit event downloaded before its target event MUST be
  stored as a pending relation, not indexed as an independent message.
- Redacted events and redacted attachments MUST be removed from the search
  index. File contents are out of scope until a separate security design is
  approved.
- Attachment filenames are searchable but confidential. They MUST follow the
  same encrypted-index, verified-highlight, and no-logging rules as message
  bodies.

## Key Management

- Generate one random local unlock secret per Matrix account and device.
- Store the local unlock secret only in the OS secret store:
  macOS Keychain on macOS, Windows Credential Manager or DPAPI on Windows.
- Do not hardcode, derive from user passwords, reuse access tokens, or commit
  local store secrets.
- Derive independent keys from the local unlock secret with domain-separated
  labels, for example one key for the SDK SQLite store and one key for the
  search index. Do not reuse the exact same key bytes for both stores.
- Missing, corrupt, or inaccessible OS secrets MUST fail closed. The app may
  offer a local-state reset flow, but it must not silently recreate keys while
  keeping unreadable encrypted data.
- Key bytes and passphrases should use zeroizing containers where practical and
  should be kept out of long-lived UI state.

## Implementation Boundaries

- Reducers and frontend state MUST NOT own SDK clients, filesystem handles,
  network clients, keyring handles, or decrypted long-lived caches.
- React/Tauri UI state may hold only the current visible snapshot needed to
  render the UI. It must not become a secondary message database.
- Backend adapters are responsible for SDK calls, key access, search indexing,
  late decryption repair, edit resolution, and redaction cleanup.
- If local search state is incomplete because downloads, decryption, or relation
  repair are pending, results must be treated as partial rather than displaying
  stale or non-visible content as authoritative.

## Tests And Fixtures

- Tests must use synthetic credentials, synthetic Matrix IDs, and synthetic event
  content unless a test is explicitly marked as manual and documents the local
  setup.
- Do not copy real room messages, real access tokens, real recovery keys, real
  attachment filenames, or production search indexes into this repository.
- Do not use real personal information in tests, fixtures, screenshots, seed
  data, examples, or docs. This includes real names, handles, email addresses,
  Matrix IDs, affiliations, institutions, workplaces, lab names, room names,
  meeting titles, agendas, notes, schedules, attachment names, URLs, and local
  home-directory paths.
- Do not transcribe user screenshots or real chats into fixtures. If a UI needs
  realistic-looking content, use short synthetic labels such as `Member 1`,
  `Synthetic Workspace`, `fixture_budget.xlsx`, and Matrix IDs under
  `example.invalid`.
- Real affiliations or institutions are prohibited in synthetic data even when
  the user mentions them in conversation. Use neutral organization labels such
  as `Synthetic Workspace` instead.
- Security-sensitive behavior needs focused tests when implemented: encrypted
  index opening, missing-key failure, edit-before-target handling, redaction
  removal, attachment filename search, and verified highlight generation.

## Licensing

- Code or design ported from Element, Seshat, Matrix Rust SDK, FluffyChat, or
  related upstream projects must preserve applicable license and copyright
  notices.
- Prefer upstreamable changes for `matrix-sdk-search`; keep local patches small,
  documented, and suitable for later feedback upstream.
