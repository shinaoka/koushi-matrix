# Repository Rules

Status: normative. This is the root durable rule book for this repository.
It applies to first-party code, docs, tests, QA automation, and integration
glue. Vendored upstream code must keep its original license and copyright
notices; local changes to vendored code must remain easy to upstream or
revert.

Last amended: 2026-06-14.

## Read Order And Authority

Read these files before changing behavior:

1. `REPOSITORY_RULES.md` - durable repository rules and prohibitions.
2. `docs/architecture/overview.md` - long-term product architecture, layer
   ownership, runtime model, security model, and QA model.
3. `docs/architecture/state-machine.md` - normative reducer state machines,
   transitions, and guards.
4. `docs/policies/engineering-rules.md` - detailed policy extension for
   secrets, logging, GUI automation, async/runtime rules, and build gates.
5. The relevant dated implementation plan under `docs/superpowers/plans/`.

`AGENTS.md` is the operational entry file. It may contain local setup,
troubleshooting, and known failure notes, but durable rules discovered there
must be promoted into this file or `docs/policies/engineering-rules.md`.

When two normative documents appear to disagree, stop and reconcile the canon
before changing code. The stricter privacy/security rule applies while the
conflict is being resolved.

## Canon-First Change Protocol

- Do not improvise undocumented Matrix behavior when code contradicts the
  canon or the canon is silent. Record the assumed behavior and the observed
  behavior, then amend the relevant canon first.
- Architecture changes amend `docs/architecture/overview.md` before code.
  Durable rule changes amend this file and, when the detailed policy changes,
  `docs/policies/engineering-rules.md` with `Last amended` bumped.
- Reducer state-machine changes amend
  `docs/architecture/state-machine.md` in the same change as the reducer and
  tests.
- QA scenario, token, artifact, or cleanup contracts amend the relevant
  `docs/qa/` document and the enforcing script in the same change.
- Canon amendments must be approved by the user or the strongest available
  model for the agent family before implementation continues. Code that
  diverges from canon must not land.

## Architecture And Ownership

- New Matrix behavior is headless-first and local-server-first. It lands in
  `matrix-desktop-core` / `matrix-desktop-state`, is verified through
  `CoreCommand` / `CoreEvent` against disposable local Conduit/Tuwunel QA,
  and only then is wired to Tauri/React. GUI-first Matrix behavior is
  prohibited.
- This headless-first rule is operationalized as two phases per work item.
  **Phase A (headless contract):** model the feature as serializable
  `AppState` / reducers and `CoreCommand` / `CoreEvent` in
  `matrix-desktop-state` / `matrix-desktop-core`, proven against disposable
  local Conduit/Tuwunel homeserver QA; exit when the relevant local core QA
  scenario is green. **Phase B (GUI wiring):** a thin Tauri/React view over
  that same Rust state; exit when the browser-headless and, where that gate
  applies, Linux virtual-display GUI tests are green. Issues are split along
  this A/B boundary, and QA tokens stay private-data-free.
- Product logic and state that decide Matrix operation semantics live in Rust:
  `matrix-desktop-state` for serializable state/reducers and
  `matrix-desktop-core` for actors, commands, events, and runtime ownership.
- React may own ephemeral presentation state only: focus, popovers, unsent form
  text, viewport measurements, virtual-list cache, and scroll anchors. If UI
  state affects a Matrix command shape, selected target, pending operation,
  cleanup, retry, or success/failure interpretation, model it first as
  serializable Rust `AppState` / `CoreEvent` data and prove it headlessly.
- `apps/desktop/src-tauri` is a transport adapter. It holds `CoreRuntime`,
  sends commands, forwards events/snapshots, and does not call Matrix SDK
  wrapper APIs directly.
- `matrix-desktop-sdk` is the low-level Matrix SDK adapter crate. It owns
  SDK-facing primitives only; app state, actor lifecycle, QA orchestration,
  and product opinions stay in `matrix-desktop-core` and
  `matrix-desktop-state`.
- UI code must not import SDK types. SDK data is mapped to app-owned Rust DTOs
  before it crosses the command/event/snapshot boundary.
- `matrix-desktop-backend` is fixture/demo only. Production Tauri paths must
  not execute fixture-backend behavior.

## State-Machine Discipline

- Every Matrix-affecting workflow is designed as an explicit guarded state
  machine: state enum/DTO, event/action enum, start guard, settle guard,
  stale-input behavior, failure behavior, terminal behavior, and request
  correlation when applicable.
- State transitions are driven by events, not by ad-hoc field assignments. An
  action describes intent or observed outcome; reducers derive the next state.
- A transition present in code but absent from
  `docs/architecture/state-machine.md`, or present in the diagram but absent
  from code, is a defect.
- Tests for state machines cover the happy path, failure path, cancellation or
  reset path, stale request IDs, duplicate completions, and invalid state
  inputs. A headless test must fail before a new transition is implemented.
- React may display state-machine state, but it must not repair product state
  after the fact. If completion of a Matrix operation should clear or advance
  product state, the Rust state machine performs that transition.
- Reducer effects or command outcomes that are part of production behavior must
  be executed by the production runtime or replaced by an explicit
  `CoreCommand`/actor path. Producing an `AppEffect` and then discarding it in a
  production path is a defect.
- Actor-to-reducer transitions that settle pending user-visible state are not
  lossy notifications. They must be delivered reliably or paired with a
  deterministic failure transition that clears/reports the pending state. Silent
  `try_send` drops are prohibited for send, reply, thread, room, search,
  cleanup, recovery, and login state machines.

## Security Rules

- Decrypted E2EE event bodies, attachment filenames, snippets, search queries,
  access tokens, refresh tokens, recovery keys, room keys, local store keys,
  SDK store keys, search index keys, and local unlock secrets are secrets.
- Persistable Matrix session JSON contains access tokens and refresh tokens.
  It is a secret even when wrapped in a redacted Rust type.
- Secrets MUST NOT be logged, sent to telemetry, written to crash reports,
  printed in test output, checked into fixtures, or copied into screenshots.
  Credential/key material must not be returned to the webview after entry.
  Decrypted message bodies and attachment filenames may cross to the webview
  only as current visible UI state; they must not become logs, diagnostics,
  fixtures, screenshots, or secondary first-party stores.
- Real account private data MUST NOT appear in docs, tests, mocks, logs,
  screenshots, or QA artifacts. This includes real room names, message bodies,
  attachment names, Matrix IDs, email addresses, institutions, workplaces,
  meeting titles, and local home-directory paths.
- Real-account and real-homeserver QA must be private-data-free before artifact
  persistence. Leak checks that run only after writing raw stdout/stderr to a
  log are insufficient; either redact before write or ensure the producer never
  formats real Matrix IDs, room/event IDs, message bodies, raw SDK errors, or
  local paths into the captured stream.
- Debug output for secret-bearing commands, actions, and errors must redact
  associated values. Enum/action logging may record the case/kind; it must not
  record passwords, tokens, recovery material, message bodies, attachment
  names, raw SDK errors, transaction IDs, event IDs from real accounts, or room
  IDs from real accounts.
- E2EE trust, verification, cross-signing, key-backup, and identity-reset
  state may carry only app-owned DTOs and private-data-free failure kinds.
  Private keys, recovery secrets, room keys, key-backup secrets, and raw SDK
  errors must never cross the command/event/snapshot boundary. Debug output and
  QA tokens for these flows must redact account keys, verification target user
  and device IDs, and backup version identifiers.
- `.local-secrets/` is reserved for local, ignored manual-testing notes or
  scratch files only. It is not an application secret store, must not be
  required for tests or builds, and must not replace OS secret storage.
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
  candidates without a verified visible span must be dropped or shown only by
  an explicitly non-exact result mode.
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
- Store the local unlock secret only in the OS secret store: macOS Keychain on
  macOS, Windows Credential Manager or DPAPI on Windows.
- Do not hardcode, derive from user passwords, reuse access tokens, or commit
  local store secrets.
- Derive independent keys from the local unlock secret with domain-separated
  labels, for example one key for the SDK SQLite store and one key for the
  search index. Do not reuse the exact same key bytes for both stores.
- Missing, corrupt, or inaccessible OS secrets MUST fail closed. The app may
  offer a local-state reset flow, but it must not silently recreate keys while
  keeping unreadable encrypted data.
- Key bytes and passphrases should use zeroizing containers where practical
  and should be kept out of long-lived UI state.

## QA Gates And Cleanup

- GUI automation is a smoke layer, not the primary correctness gate. React UI,
  command shapes, fake `CoreEvent` streams, DOM scroll behavior, and Tauri IPC
  mock behavior are verified in headless browser tests first.
- Destructive GUI operation QA during development uses disposable local
  Conduit/Tuwunel homeservers. Do not use matrix.org or another real
  homeserver for GUI iteration.
- Real homeserver QA is reserved for compatibility and release/preflight gates
  after local headless and Linux virtual-display lanes are green and cleanup
  behavior is proven.
- QA scripts must assert scenario-specific success tokens, not only process
  exit code. If a document promises a token, the script enforces it or the
  document is wrong.
- Real-account and real-homeserver QA diagnostics are tokenized. Failure output
  may identify the failed step and coarse failure kind, but must not include raw
  Matrix IDs, transaction IDs, event IDs, room IDs, user IDs, SDK error strings,
  message bodies, search queries, or local filesystem paths.
- Real-account and real-homeserver QA must use cleanup guards for every
  resource it creates: sessions/devices, rooms, spaces, memberships, stores,
  search indexes, and background processes. After the first post-login side
  effect, early `?` returns are allowed only inside a cleanup guard that still
  attempts logout and resource cleanup unless `--keep-session` was explicitly
  requested.
- QA runners must not pass the parent shell environment wholesale into child
  processes. Filter secret-like variables before spawning.
- QA credentials enter processes through FIFO or the debug/test-gated file
  credential store. They must not appear in argv, fixed-coordinate typing,
  screenshots, committed scripts, or captured terminal output.

## Tests And Fixtures

- Tests must use synthetic credentials, synthetic Matrix IDs, and synthetic
  event content unless a test is explicitly marked as manual and documents the
  local setup.
- Manual live-login smoke checks must collect real credentials interactively or
  through an approved secret-minimized QA pipe. Do not pass real usernames,
  passwords, recovery keys, or access tokens through command-line arguments,
  environment variables, fixtures, committed scripts, or captured test output.
- Do not copy real room messages, real access tokens, real recovery keys, real
  attachment filenames, or production search indexes into this repository.
- Do not use real personal information in tests, fixtures, screenshots, seed
  data, examples, or docs. Use neutral examples such as `Member 1`,
  `Synthetic Workspace`, `fixture_budget.xlsx`, and Matrix IDs under
  `example.invalid`.
- Do not transcribe user screenshots or real chats into fixtures. If a UI
  needs realistic-looking content, synthesize it.
- Real affiliations or institutions are prohibited in synthetic data even when
  the user mentions them in conversation. Use neutral organization labels such
  as `Synthetic Workspace` instead.
- Security-sensitive behavior needs focused tests when implemented: encrypted
  index opening, missing-key failure, edit-before-target handling, redaction
  removal, attachment filename search, verified highlight generation,
  credential gate rejection, private-data-free QA title generation, and DTO
  snapshot completeness.

## User-Facing Text And Localization

- User-visible product text must not be embedded directly in React components,
  Rust core errors, Tauri commands, or tests that model production UI. Use a
  message catalog keyed by stable IDs, with interpolation for dynamic values.
- English-only product copy is acceptable while the localization system is
  still small, but it still goes through the catalog. Deferring Japanese or
  other locale quality does not permit hardcoded English/Japanese strings in
  product UI, menus, accessibility labels, placeholders, empty states, dialogs,
  or validation messages.
- Core and adapters return machine-readable kinds, codes, and structured
  non-secret data. They do not return English/Japanese prose for the UI to
  display, except for debug/test-only diagnostics.
- The UI boundary is responsible for resolving message IDs to localized text.
  Accessibility labels, button labels, menu labels, empty states, dialogs,
  toasts, and validation messages are user-facing text and use the same
  catalog.
- Locale/display behavior is Rust-owned. GUI code consumes the resolved
  `LocaleDisplayProfile` (`lang`, `dir`, catalog locale, pseudo-locale mode,
  platform, and modifier labels) and must not branch on raw persisted locale
  tags in feature components.
- `LocaleDisplayProfile` is part of the snapshot contract. Changes to it must
  update the Tauri DTO, TypeScript domain types, browser fake snapshots, Tauri
  IPC mocks, app harness snapshots, and DTO serialization-contract tests
  together.
- QA tokens, protocol enum variants, log kinds, CSS class names, data-testid
  values, and synthetic fixture message bodies are not localized. Tests should
  prefer roles, stable test IDs, message IDs, or semantic state over localized
  prose when possible.
- A new feature that adds user-visible text adds or updates catalog entries,
  at least one default locale, and a pseudo-locale or missing-translation test
  before wiring the text into the UI.
- Locale-sensitive layout uses CSS logical properties by default. Unreviewed
  physical left/right spacing, borders, positioning, or text alignment in the
  desktop shell is a defect unless the physical direction is intentional and
  documented.
- CJK text fitting is a presentation contract. GUI code may use CSS line-break,
  word-break, hyphenation, wrapping, and ellipsis rules to fit Rust-owned room
  names, sender/member names, message bodies, thread labels, and snippets, but
  must not rewrite text, recompute sort keys, normalize queries, or repair
  highlights locally.

## Documentation And Work Records

- Dated implementation plans are subordinate to the normative docs. When an
  implementation discovery changes architecture or rules, amend the canon
  first, then sync or supersede the dated plan.
- Nontrivial agent-driven work should leave a short plan, worklog, review
  record, or changelog entry that identifies the canon consulted, the files
  changed, and the verification run. This is required when changing state
  machines, security behavior, SDK fork surfaces, QA gates, or release gates.
- Operational setup and failure notes stay in `AGENTS.md` until they become
  durable prohibitions or design rules.

## Licensing

- Code or design ported from Element, Seshat, Matrix Rust SDK, FluffyChat, or
  related upstream projects must preserve applicable license and copyright
  notices.
- Prefer upstreamable changes for `matrix-sdk-search` and the vendored Matrix
  Rust SDK. Keep local patches small, documented, and suitable for later
  feedback upstream.
