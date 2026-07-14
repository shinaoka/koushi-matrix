# Repository Rules

Status: normative. This is the root durable rule book for this repository.
It applies to first-party code, docs, tests, QA automation, and integration
glue. Vendored upstream code must keep its original license and copyright
notices; local changes to vendored code must remain easy to upstream or
revert.

Last amended: 2026-07-03.

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

## Root-Cause Fix Discipline

- Permanent fixes must address the authoritative cause, not only the visible
  symptom. When a bug exposes a mismatch between Rust state/reducers, core
  actors, Tauri commands, React rendering, browser fakes, QA scripts, or docs,
  fix the owning contract first and align adapters, fixtures, and tests to that
  contract.
- Ad hoc patches, UI-only repairs, fixture-only behavior, extra waits, or
  expectation relaxations are allowed only as temporary diagnostic or
  containment steps. They must be called out as temporary, tracked in the
  relevant plan/worklog/issue, and removed or replaced by the root-cause fix
  before phase exit, release gating, or landing on `main`.
- A review finding that can be fixed either by masking the failure or by
  correcting the underlying state machine, command/event contract, SDK adapter,
  or QA oracle must choose the underlying correction. If that correction
  requires a canon change, follow the Canon-First Change Protocol before code
  continues.

## Architecture And Ownership

- New Matrix behavior is headless-first and local-server-first. It lands in
  `koushi-core` / `koushi-state`, is verified through
  `CoreCommand` / `CoreEvent` against disposable local Conduit/Tuwunel QA,
  and only then is wired to Tauri/React. GUI-first Matrix behavior is
  prohibited.
- Before designing or implementing new user-visible Matrix functionality,
  inspect the corresponding Element Web and Element X Android/iOS behavior
  when those clients have an equivalent flow. Record the observed upstream
  command/state shape and UX flow in the issue, plan, or PR notes, and call out
  any intentional divergence before code lands. If Element and Element X differ,
  prefer the behavior that matches Matrix semantics and desktop user
  expectations, with the tradeoff documented.
- This headless-first rule is operationalized as two phases per work item.
  **Phase A (headless contract):** model the feature as serializable
  `AppState` / reducers and `CoreCommand` / `CoreEvent` in
  `koushi-state` / `koushi-core`, proven against disposable
  local Conduit/Tuwunel homeserver QA; exit when the relevant local core QA
  scenario is green. **Phase B (GUI wiring):** a thin Tauri/React view over
  that same Rust state; exit when the browser-headless and, where that gate
  applies, Linux virtual-display GUI tests are green. Issues are split along
  this A/B boundary, and QA tokens stay private-data-free.
- Product logic and state that decide Matrix operation semantics live in Rust:
  `koushi-state` for serializable state/reducers and
  `koushi-core` for actors, commands, events, and runtime ownership.
- React may own ephemeral presentation state only: focus, popovers, unsent form
  text, viewport measurements, virtual-list cache, and scroll anchors. If UI
  state affects a Matrix command shape, selected target, pending operation,
  cleanup, retry, or success/failure interpretation, model it first as
  serializable Rust `AppState` / `CoreEvent` data and prove it headlessly.
- `apps/desktop/src-tauri` is a transport adapter. It holds `CoreRuntime`,
  sends commands, forwards events/snapshots, and does not call Matrix SDK
  wrapper APIs directly.
- `koushi-sdk` is the low-level Matrix SDK adapter crate. It owns
  SDK-facing primitives only; app state, actor lifecycle, QA orchestration,
  and product opinions stay in `koushi-core` and
  `koushi-state`.
- UI code must not import SDK types. SDK data is mapped to app-owned Rust DTOs
  before it crosses the command/event/snapshot boundary.
- `koushi-backend` is fixture/demo only. Production Tauri paths must
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
- Background workers that consume authoritative latest snapshots, such as the
  search history crawler's joined-room availability notification, must not block
  user-visible actor commands. They may use latest-wins coalescing and
  nonblocking `try_send` only when the owning actor retains the newest pending
  payload and retries it later; dropping the only pending update silently is
  still prohibited.
- Pane-level thread attention is a Rust-owned read-state projection, not a
  timeline-vector projection. It uses the authoritative threaded receipt when
  available, explicit hydration/live/backfill/replay lifecycle, matching
  `m.thread` relations, and stable event-ID deduplication. Relay batches carry
  SDK event-origin provenance; consumers must not reconstruct lifecycle from
  ambient pagination/task state. A vector mutation such as `PushBack` is never
  sufficient evidence that a reply is new. Thread
  summary total reply counts remain separate from new/unread attention, and a
  successful threaded read acknowledgement clears attention through the Rust
  actor/reducer path.

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
- Public command/event/snapshot DTOs must make their `Debug` privacy contract
  explicit. Derive `Debug` only for types whose fields are all safe to paste
  into CI logs, QA artifacts, and GitHub issues. If a public type carries
  private content, Matrix identifiers, local paths, raw SDK diagnostics, or
  plaintext-derived snippets, implement custom redacted `Debug` that exposes
  only kinds, booleans, counts, and placeholders.
- E2EE trust, verification, cross-signing, key-backup, and identity-reset
  state may carry only app-owned DTOs and private-data-free failure kinds.
  Private keys, recovery secrets, room keys, key-backup secrets, and raw SDK
  errors must never cross the command/event/snapshot boundary. Debug output and
  QA tokens for these flows must redact account keys, verification target user
  and device IDs, and backup version identifiers.
- Manual room-key file export/import MUST use the Matrix key-export file
  format that Element clients use, including the encrypted Megolm session data
  header/footer handled by the public Matrix Rust SDK APIs. Do not introduce a
  product-specific JSON, archive, or wrapper file format for room-key transfer.
  Tests for this flow must use synthetic fixtures and assert interoperability
  without logging or snapshotting room-key file contents.
  If the public SDK export API does not return an exported-session count,
  reducer/DTO state must represent that count as unknown instead of decrypting,
  parsing, or re-wrapping the export file only to derive UI metadata.
- `.local-secrets/` is reserved for local, ignored manual-testing notes or
  scratch files only. It is not an application secret store, must not be
  required for tests or builds, and must not replace OS secret storage.
- Decrypted event bodies and plaintext-derived data MUST NOT be persisted in
  first-party stores outside an encrypted Matrix SDK store or encrypted search
  index.
- Read-receipt reader avatars are Rust-owned live-signal projection data.
  Reducers/core must resolve reader labels/avatar DTOs, most-recent-first
  ordering, cap, and overflow count before the snapshot reaches React. GUI code
  may render that projection and provide ephemeral tooltip visibility only; it
  must not join receipt user ids with profile maps, choose receipt ordering, or
  print real reader names/avatar MXC URIs in QA evidence.
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
- Rust-projected identity fields such as `display_label` and
  `original_display_label` are dynamic room/user data, not localized product
  prose. They must resolve from real alias/upstream/profile/MXID/room-id data;
  never use generic hardcoded labels such as `Member` as identity fallbacks.
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

## Product Identity And Migration

- The shipped product name is **Koushi**. User-facing strings, window titles,
  installer metadata, docs, and QA artifacts must use Koushi. The GitHub
  repository is `shinaoka/koushi-matrix`.
- Current internal identifiers are:
  - Internal crate/module prefix: `koushi-*`
  - Tauri bundle identifier: `chat.koushi.desktop`
  - npm/Cargo package name: `koushi-desktop`
  - keychain / file credential-store service name: `koushi-desktop`
  - Matrix global account-data key for local user aliases:
    `app.koushi.local_aliases`
- No compatibility migration from old Matrix Desktop/Kagome identifiers is
  required at this stage because there is no supported persisted user data to
  preserve. New storage, QA env vars, keychain entries, account-data keys, and
  internal app event schemes must use Koushi identifiers. Renaming these
  identifiers again requires an explicit migration plan and user approval.

## Concurrent Work And Merge-Conflict Avoidance

Wave 2 (#38 device/session manager and #39 sliding-sync/room-list filters)
confirmed that parallel Phase A work collides on shared surfaces when agents
treat them as free-form append targets. The following rules reduce those
conflicts without weakening the serialization points that keep the stack
consistent.

### Test Placement

- **Integration-style or projection tests belong under `tests/` per feature.**
  Tests that exercise reducer/command/event/runtime projection, DTO snapshots,
  or state-machine transitions are integration-style. Put them in
  `crates/<crate>/tests/<feature>.rs`, not inside a monolithic
  `#[cfg(test)] mod tests` block in a source file.
- **Pure unit tests may stay inline.** Small tests for a single pure helper,
  parser, or private algorithm may remain in the source file under
  `#[cfg(test)] mod tests`. When a unit test file grows beyond one screen or
  begins to assert cross-module projection, move it to `tests/`.
- **Do not add new tests to existing monolithic test files such as
  `crates/koushi-core/src/tests.rs`.** Add a new `tests/<feature>.rs`
  file instead. Existing monolithic files may be split opportunistically when
  they are touched for a new feature.
- **Test fixtures and fakes belong near their consumer.** A fake used by a
  single feature's tests lives in that feature's test module. Shared fakes live
  in `src/test_support.rs` or `tests/support/` and must be append-friendly.

### Shared Hot Files

The main agent owns integration of the following shared surfaces. Subagents may
read them but must not append to them without main-agent coordination:

- `crates/koushi-state/src/{state.rs,action.rs,reducer.rs}`
- `crates/koushi-core/src/{command.rs,event.rs,runtime.rs}`
- `apps/desktop/src-tauri/src/{dto.rs,commands.rs}`
- `apps/desktop/src/{App.tsx,components/TimelineView.tsx,i18n/messages.ts,styles.css}`
- `apps/desktop/src/domain/{types.ts,coreEvents.ts,coreEvents.generated.json}`
- Browser-headless GUI-operation specs and Tauri IPC mocks

To reduce conflicts on these files:

- **Group related fields into nested structs/DTOs.** Instead of adding several
  top-level fields to `AppState` for one feature, add one nested struct
  (e.g., `AppState.account_management`, `AppState.room_list`). This confines
  most feature-specific diffs to the nested type and its mirror on the
  TypeScript side.
- **Avoid central re-export lists that every feature edits.** When a crate's
  `lib.rs` becomes a long list of per-feature re-exports, prefer re-exporting
  the feature module namespace (`pub mod account_management;`) or a
  feature-grouped prelude. Each feature then edits its own module's public API
  rather than a shared list.
- **Keep generated contract artifacts append-friendly.** Additions to
  `coreEvents.generated.json` go at the end of the relevant array/object
  without renumbering or reformatting unrelated entries. Do not regenerate the
  artifact with unrelated formatting churn in the same change.

### Parallel Implementation Protocol

- **Serialize shared surface design before parallelizing implementation.** The
  main agent must decide module boundaries, enum variants, nested DTO shapes,
  and test file names before subagents begin coding. Subagents receive a bounded
  file allow-list and a shared-file deny-list in their prompt.
- **Do not parallelize two agents on the same hot file.** Cap concurrent
  subagents to disjoint territories (typically 2-3). If two features both need
  to change the same hot file, either split the work sequentially or have the
  main agent pre-apply the shared scaffold and let subagents fill module-local
  bodies.
- **Subagent output is a draft to integrate, not merged evidence.** Cheap
  implementation agents may write module-local code, tests, and docs. The main
  agent still integrates shared enums, reducers, command/event variants, Tauri
  DTOs, TypeScript wire, generated contract artifacts, and issue comments.
- **Merge integration branches before landing on `main`.** When multiple feature
  branches run in parallel, create a short-lived integration worktree, resolve
  conflicts and run the full gate there, then fast-forward `main`. Do not push
  a feature branch directly to `main` while another parallel feature is still
  open.

### Worktree And Build Artifact Cleanup

- **Remove temporary worktrees as soon as they are no longer needed.** A feature
  branch that has been merged into `main` or into an integration branch should
  not keep a worktree alive. Use `git worktree remove --force <path>` when the
  worktree contains submodules.
- **Delete worktree-local build intermediates before or during worktree removal.**
  Worktrees often accumulate large per-worktree artifacts:
  - Rust build artifacts under the worktree's `target/` directory when
    `CARGO_TARGET_DIR` is not shared or when the worktree overrides it.
  - Vite/Tauri dev caches under `apps/desktop/node_modules/.vite/`.
  - Per-worktree `node_modules/` when the worktree does not share the main
    workspace's dependency tree.
  These must be cleaned up because they can consume many gigabytes and are not
  needed after the branch is merged.
- **Do not delete shared build directories.** When `CARGO_TARGET_DIR` points to a
  shared location (e.g., the main workspace `target/`), confirm that the
  directory is shared across worktrees before deleting it. Shared target
  directories speed up rebuilds and must be preserved.
- **Verify cleanup.** After removing a worktree, confirm `git worktree list` and
  disk usage look reasonable. Large leftover artifacts are a hygiene defect.

## Review And Audit

- **Non-frontier models must receive frontier-model review for substantial work.**
  When a cheaper or non-frontier agent (e.g., a fast implementation subagent)
  completes a significant change — especially after parallel implementation,
  AgentSwarm work, or changes to shared hot files — a frontier model must
  review the diff against the canon (`REPOSITORY_RULES.md`,
  `docs/architecture/overview.md`, `docs/architecture/state-machine.md` when
  reducers change, `docs/policies/engineering-rules.md`, `AGENTS.md`, and the
  relevant dated plan). The review must include the verification output and
  any security/privacy-sensitive surfaces.
- **Prefer `codex` as the auditor.** When scheduling an external review, use
  the `codex` CLI as the first choice. Other frontier models may be used only
  when `codex` is unavailable or explicitly declined by the user.
- **Review focus areas.** The auditor must prioritize, in order:
  1. Consistency with repository rules and canon documents.
  2. Consistency with Rust/Tauri best practices and the existing codebase.
  3. Security and privacy risks, including secret leakage, unsafe code,
     untrusted input handling, and cross-boundary data exposure.
  4. Correctness of state-machine, command/event, and DTO contracts.
- **Rule gaps found during audit must be reported as rule-update proposals.**
  If the auditor discovers a problem that is caused or enabled by a gap,
  ambiguity, or missing rule in the canon, the review must propose an
  amendment to `REPOSITORY_RULES.md` or `docs/policies/engineering-rules.md`
  rather than only patching the immediate code. The main agent decides
  whether to adopt the proposal, escalate to the user, or defer it.
- **Review findings are implementation tasks, not optional suggestions.** The
  implementing agent or the main agent must address blocking issues and
  re-run the relevant gates before landing the change on `main`.
- **Frontier-model-authored implementation is exempt from mandatory external
  review**, but the author should still run the full gate set and perform a
  self-audit before claiming completion. If the frontier model is uncertain
  about a cross-boundary decision, it must escalate to the user or pause for
  review before proceeding.
- **Audit scope is proportional to risk.** A narrow module-local patch may
  need only a quick diff check; a parallel Phase A integration that touches
  shared enums, reducers, command/event variants, Tauri DTOs, TypeScript wire,
  and generated contracts needs a thorough cross-boundary audit.
- **Keep review prompts scoped and private-data-free.** Review prompts must
  include only synthetic fixture data; real account credentials, room IDs,
  event IDs, message bodies, raw SDK errors, and local paths must never be
  sent to an external review model.

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
