# Repository Rules

Status: normative. This is the root durable rule book for this repository and
is read on every task. It applies to first-party code, docs, tests, QA
automation, and integration glue. Vendored upstream code must keep its original
license and copyright notices; local changes to vendored code must remain easy
to upstream or revert.

## Read Order And Authority

- Read this file first. Then read only the applicable sections of
  `docs/architecture/overview.md` (layers/runtime),
  `docs/architecture/state-machine.md` (transitions and guards),
  `docs/architecture/i18n.md` (product text and locale/display), and
  `docs/policies/engineering-rules.md` (detailed security, runtime, and gate
  policy); use `docs/agents/plans.md` to find a governing dated plan. Questions
  and documentation-only edits need the applicable contracts, not unrelated
  feature specifications.
- `AGENTS.md` is the router, small enough to load every session; `docs/agents/`
  carries setup, lane, ownership, troubleshooting, and historical detail.
  Durable rules discovered there are promoted into this file or the engineering
  rules.
- When two normative documents appear to disagree, stop and reconcile the canon
  before changing code. The stricter privacy/security rule applies meanwhile.

## Canon-First Change Protocol

- Do not improvise undocumented Matrix behavior when code contradicts the
  canon or the canon is silent. Record the assumed and observed behavior, then
  amend the relevant canon first.
- Architecture changes amend `docs/architecture/overview.md` before code; dated
  specs and plans implement it. Durable rule changes amend this file and, when
  the detailed policy changes, the engineering rules.
- Reducer state-machine changes amend `docs/architecture/state-machine.md` in
  the same change as the reducer and tests.
- QA scenario, token, artifact, or cleanup contracts amend
  `docs/agents/qa-lanes.md` (and the matching `docs/qa/` document when one
  exists) and the enforcing script in the same change.
- Canon amendments must be approved by the user, or through the risk-based
  review in [Review And Audit](#review-and-audit), before implementation
  continues. Code that diverges from canon must not land.

## Root-Cause Fix Discipline

- Permanent fixes address the authoritative cause, not only the visible
  symptom. When a bug exposes a mismatch between Rust state/reducers, core
  actors, Tauri commands, React rendering, browser fakes, QA scripts, or docs,
  fix the owning contract first and align adapters, fixtures, and tests to it.
- Ad hoc patches, UI-only repairs, fixture-only behavior, extra waits, or
  expectation relaxations are allowed only as temporary diagnostic or
  containment steps: called out as temporary, tracked in the plan/worklog/issue,
  and replaced by the root-cause fix before phase exit, release gating, or
  landing on `main`.
- When a review finding can be fixed by masking the failure or by correcting the
  underlying state machine, command/event contract, SDK adapter, or QA oracle,
  choose the correction, following the Canon-First Change Protocol if it needs a
  canon change.

## Text Input And IME Ownership

- Every user-editable text surface in the desktop React application uses the
  shared IME-safe text-control primitives; raw composable `input`, `textarea`,
  `form`, and unapproved `contentEditable` are prohibited outside them. The DOM
  owns active composition and unacknowledged drafts, candidate-confirmation
  Enter is never a product command, async text writes are latest-wins per
  field, and entered secrets stay DOM-owned. Details and the lint gate:
  engineering rules "Desktop Text Input And IME Safety".

## User-Visible Text

- Visible action labels use the shortest wording that stays unambiguous in
  context: when the target or mode is already clear, use the verb alone
  (`Accept`, `Decline`, `Cancel`, `Save`), not `Accept invite` or `Cancel edit`.
  Keep a descriptive accessible name when visible context is insufficient for
  assistive technology.

## Property Display And Editing

- One property has one primary place: its current value, the control that
  changes or clears it, and the result of that change live together, in every
  state (unset, read-only, in flight, failed). New or changed UI must not
  introduce a violation. Full rules: engineering rules "Property Display And
  Editing".

## macOS Native Window Controls And Overlay Layout

- Preserve the standard macOS window buttons and keep every in-app surface's
  content and controls below them through the shared safe-area layout
  primitives; never hide or cover the native buttons, and never impose the inset
  on Windows or Linux. Full rules: engineering rules "macOS Native Window
  Controls And Overlay Layout".

## Architecture And Ownership

- The checked-out `vendor/matrix-rust-sdk` submodule is the authoritative Matrix
  Rust SDK source; root workspace SDK dependencies use exact paths beneath it,
  the gitlink is the single revision pin, and `scripts/check-sdk-submodule.mjs`
  stays green (engineering rules "Build, Dependencies, QA Gates" item 1).
- New Matrix behavior is headless-first and local-server-first; GUI-first Matrix
  behavior is prohibited. **Phase A (headless contract):** model the feature as
  serializable `AppState`/reducers and `CoreCommand`/`CoreEvent` in
  `koushi-state`/`koushi-core`, proven against disposable local Tuwunel/Synapse
  QA through the non-default `koushi-qa` package; exit when the relevant local
  core QA scenario is green. **Phase B (GUI wiring):** a thin Tauri/React view
  over that same Rust state; exit when the browser-headless and, where that gate
  applies, Linux virtual-display GUI tests are green. Issues are split along this
  A/B boundary, and QA tokens stay private-data-free.
- The desktop runtime has exactly one sync engine: Element X-compatible
  Simplified Sliding Sync. Legacy `/sync`, backend probing/forcing, fallback
  selection, and backend-selection state are not supported in product, QA, or
  diagnostics code. Local QA accepts only `--server=tuwunel`,
  `--server=synapse`, or `--server=both` and rejects `--core-backend` and
  `KOUSHI_QA_FORCE_SYNC_BACKEND`.
- A `VectorDiff` accumulator is advanced only by diffs from the stream that owns
  it, and its projection claims authority only when entry count equals
  distinct-identity count (engineering rules "Crate Ownership And Projection
  Authority").
- Before designing or implementing new user-visible Matrix functionality,
  inspect the equivalent
  Element Web and Element X Android/iOS flow, record the observed upstream
  command/state shape and UX in the issue, plan, or PR notes, and call out any
  intentional divergence before code lands. If they differ, prefer the behavior
  that matches Matrix semantics and desktop expectations, documenting the
  tradeoff.
- Product logic and state that decide Matrix semantics live in Rust:
  `koushi-state` (serializable state/reducers) and `koushi-core` (actors, policy,
  projection, runtime ownership). `koushi-protocol` owns the public
  transport-neutral command/event/identity/failure/state-update DTOs and has no
  Matrix SDK, Tauri, async-runtime, filesystem/platform, or OS dependency;
  secret-bearing typed commands are built by each validated adapter and need not
  be wholesale serde payloads.
- The toolkit-independent Rust core is the sole reusable owner of product
  semantics and authoritative state for every renderer and host. React/Tauri and
  future native adapters may own transport, layout, focus, IME composition, and
  platform capabilities, but must not add a competing semantic state machine,
  retry/resource policy, or toolkit-dependent product contract. Touched legacy
  owners move toward this boundary and are removed with their migration slice;
  this does not require shipping future renderers.
- React may own ephemeral presentation state only: focus, popovers, unsent form
  text, viewport measurements, virtual-list cache, and scroll anchors. UI state
  that affects a Matrix command shape, selected target, pending operation,
  cleanup, retry, or success/failure interpretation is modelled first as
  serializable Rust `AppState`/`CoreEvent` data and proven headlessly.
- Product and async lifecycle state that can outlive one mounted DOM owner
  belongs to Rust: Core actors retain, cancel, and await every subscription and
  task they create (engineering rules "Async and Runtime" 2). React may own a
  listener, observer, frame, or timer only for presentation lifetime; product
  retries, backoff, correlation, and session cleanup never use browser timers or
  refs.
- WebView `localStorage` is not a product-state or preference store; production
  code touches it only in the allowlisted legacy migration reader (engineering
  rules "Secrets and Private Data" 10).
- Browser and component tests use explicit Rust-shaped snapshots/events plus
  transport fixtures. A frontend fake must not implement reducer, actor,
  projection, search-matching, composer-resolution, ordering, retry, or terminal
  semantics that could let the browser tier pass against a second state machine;
  tests for those contracts belong in Rust.
- `apps/desktop/src-tauri` is a transport/platform adapter: it holds
  `CoreRuntime`, constructs `koushi-protocol` commands, forwards events and
  snapshots, and never calls Matrix SDK wrapper APIs directly. Platform URI
  minting and native artifact-path registration stay there; Core/state/protocol
  expose only opaque references and typed ports.
- Every public `#[tauri::command]` in `apps/desktop/src-tauri/src/commands/` is
  registered in `tauri::generate_handler!` in `apps/desktop/src-tauri/src/lib.rs`;
  an unregistered command compiles yet never reaches Rust. The exhaustive guard
  is rule `desktop.commands.tauri_command_registration`
  (`checkDesktopTauriCommandRegistrationContract()` in
  `scripts/check-rust-test-structure.mjs`), run by the CI step "Rust test
  structure checker"; keep it green.
- Crate ownership follows engineering rules "Crate Ownership And Projection Authority": `koushi-sdk`,
  `koushi-store`, `koushi-search`, `koushi-media`, `koushi-qa`, and
  `koushi-core-testkit` own only their stated leaf concerns, and product policy
  stays in Core.
- UI code must not import SDK types; SDK data is mapped to app-owned Rust DTOs
  before it crosses the command/event/snapshot boundary.

## State-Machine Discipline

- Every Matrix-affecting workflow is an explicit guarded state machine: state
  enum/DTO, event/action enum, start guard, settle guard, stale-input, failure,
  and terminal behavior, and request correlation when applicable. Transitions
  are driven by events, not ad-hoc field assignments.
- Every reducer state machine, and actor-owned product state exposed through
  DTOs or guarded commands, is documented in
  `docs/architecture/state-machine.md`. A transition in code but not in that
  document, or the reverse, is a defect.
- State-machine tests cover the happy, failure, cancellation/reset, stale
  request ID, duplicate completion, and invalid-input paths. A headless test must
  fail before a new transition is implemented.
- React may display state-machine state but must not repair product state after
  the fact; the Rust state machine performs completion transitions.
- Production reducer effects are executed by the production runtime or replaced
  by an explicit `CoreCommand`/actor path; discarding a production `AppEffect` is
  a defect.
- Actor-to-reducer transitions that settle pending user-visible state are
  delivered reliably or paired with a deterministic failure transition. Silent
  `try_send` drops are prohibited for send, reply, thread, room, search, cleanup,
  recovery, and login state machines; the permitted nonblocking cases are in
  overview "Async Design Rules" (delivery discipline).

## Security Rules

- Secrets: decrypted E2EE bodies, attachment filenames, snippets, search
  queries, access and refresh tokens, persistable Matrix session JSON (even in a
  redacted wrapper), recovery keys, room keys, local/SDK store keys, search index
  keys, and local unlock secrets.
- Secrets MUST NOT be logged, sent to telemetry, written to crash reports,
  printed in test output, checked into fixtures, or copied into screenshots.
  Credential/key material is not returned to the webview after entry; the single
  exception is a freshly generated Secure Backup recovery key in the live
  reveal state of
  [engineering rule 11](docs/policies/engineering-rules.md#secrets-and-private-data),
  never entered input echoed back. Decrypted bodies and attachment filenames
  cross to the webview only as current visible UI state, never as logs,
  diagnostics, fixtures, screenshots, or secondary first-party stores.
- Real account private data MUST NOT appear in docs, tests, mocks, logs,
  screenshots, or QA artifacts: real room names, message bodies, attachment
  names, Matrix IDs, email addresses, institutions, workplaces, meeting titles,
  or local home-directory paths.
- Real-account and real-homeserver QA output is private-data-free before
  artifact persistence; redact before write rather than leak-check afterwards
  (engineering rules "Logging and Diagnostics" 5).
- Debug output for secret-bearing commands, actions, and errors redacts
  associated values: enum/action logging may record the case/kind only, never
  passwords, tokens, recovery material, bodies, attachment names, raw SDK errors,
  transaction IDs, or real-account event/room IDs. Public DTO `Debug` contracts
  follow engineering rules "Logging and Diagnostics" 3.
- E2EE trust, verification, cross-signing, key-backup, and identity-reset state
  carry only app-owned DTOs and private-data-free failure kinds. Private keys,
  recovery secrets, room keys, key-backup secrets, and raw SDK errors never cross
  the command/event/snapshot boundary, except the reveal-state recovery key
  above; Debug and QA tokens redact account keys, verification target user and
  device IDs, and backup versions.
- Manual room-key export/import MUST use the Element-compatible Matrix
  key-export format (engineering rules "Search Index And Room-Key Export").
- `.local-secrets/` holds only ignored manual-testing notes or scratch files; it
  is not an application secret store, must not be required by tests or builds,
  and must not replace OS secret storage.
- Decrypted event bodies and plaintext-derived data MUST NOT be persisted in
  first-party stores outside an encrypted Matrix SDK store or encrypted search
  index.
- Persistent search for E2EE rooms MUST use an encrypted `matrix-sdk-search`
  index that is never the display source of truth; its plaintext-derived data is
  as confidential as the message text (engineering rules "Search Index And
  Room-Key Export").

## Key Management

- Generate one random local unlock secret per Matrix account and device, and
  store it only in the OS secret store: macOS Keychain on macOS, Windows
  Credential Manager on Windows, and the freedesktop Secret Service (for
  example GNOME Keyring or KWallet) on Linux.
- Do not hardcode, derive from user passwords, reuse access tokens, or commit
  local store secrets.
- Derive independent keys from the unlock secret with domain-separated labels
  (for example one for the SDK SQLite store and one for the search index); never
  reuse the same key bytes for both stores.
- Missing, corrupt, or inaccessible OS secrets MUST fail closed. A local-state
  reset flow may be offered, but keys are never silently recreated while
  unreadable encrypted data is kept.
- Authentication is persistent-store-first: a crypto-capable client is built on
  its encrypted persistent SDK store before any login, restore, or reauth
  activates E2EE, and a memory-store login plus session transplant is
  prohibited. A saved device ID is reused only after a non-creating preflight of
  its crypto store and device keys; any missing, mismatched, or unknown state
  fails closed and never creates replacement crypto. Fresh-login stores are
  journaled before network authorization and cleaned up only explicitly and
  exact-root. Details: overview "Runtime Model" and state machines.
- Key bytes and passphrases should use zeroizing containers where practical and
  should be kept out of long-lived UI state.
- Standard outbound Megolm pre-share, identical to Element X / stock
  matrix-rust-sdk, is the sole production send path. Koushi adds no readiness
  fence, duplicate or post-send re-share, index-0 API, recipient ledger, repair
  timer, or manual share/repair control; the only debugging control is stock
  `Room::discard_room_key()`. Details: overview "Initial outbound Megolm
  delivery".

## QA Gates And Cleanup

- GUI automation is a smoke layer, not the primary correctness gate. React UI,
  command shapes, fake `CoreEvent` streams, DOM scroll behavior, and Tauri IPC
  mocks are verified in headless browser tests first.
- Destructive GUI QA during development uses disposable local Tuwunel/Synapse
  homeservers, never matrix.org or another real homeserver.
- Real homeserver QA is a compatibility and release/preflight gate
  (engineering rules "Build, Dependencies, QA Gates" 4).
- QA scripts assert scenario-specific success tokens, not only exit codes. If a
  document promises a token, the script enforces it or the document is wrong.
- QA binaries attempt logout cleanup after any post-login failure unless
  `--keep-session` was explicitly requested.
- Real-account and real-homeserver QA uses cleanup guards for every resource it
  creates (sessions/devices, rooms, spaces, memberships, stores, search indexes,
  background processes). After the first post-login side effect, early returns
  are allowed only inside a guard that still attempts logout and cleanup unless
  `--keep-session` was explicitly requested.
- QA runners clean up their full process group on failure or interruption.
- QA credentials enter only through the FIFO or the debug/test-gated file
  credential store, and child processes never inherit the unfiltered parent
  environment (engineering rules "Secrets and Private Data" 3-4).

## Tests And Fixtures

- Tests use synthetic credentials, Matrix IDs, and event content unless a test
  is explicitly marked manual and documents its local setup.
- Tests, fixtures, screenshots, seed data, examples, and docs never use real
  personal information; use neutral examples such as `Member 1`,
  `Synthetic Workspace`, `fixture_budget.xlsx`, and Matrix IDs under
  `example.invalid`. Never copy real room messages, tokens, recovery keys,
  attachment filenames, or production search indexes into the repository, and
  never transcribe user screenshots or real chats. Real affiliations or
  institutions are prohibited in synthetic data even when the user mentions
  them.
- Manual live-login smoke collects real credentials interactively or through an
  approved secret-minimized QA pipe, never through argv, environment variables,
  fixtures, committed scripts, or captured output.
- Security-sensitive behavior gets focused tests: encrypted index opening,
  missing-key failure, edit-before-target handling, redaction removal,
  attachment filename search, verified highlights, credential gate rejection,
  private-data-free QA titles, and DTO snapshot completeness.

## User-Facing Text And Localization

- User-visible product text goes through the message catalog, even while copy is
  English-only; Core and adapters return kinds and codes, not prose.
  Locale/display behavior is Rust-owned, layout uses CSS logical properties, and
  CJK fitting is CSS-only presentation. Full rules: engineering rules "Product
  Text And Localization" and `docs/architecture/i18n.md`.

## Product Identity And Migration

- The shipped product name is **Koushi**; user-facing strings, window titles,
  installer metadata, docs, and QA artifacts use it. The GitHub repository is
  `shinaoka/koushi-matrix`.
- Current internal identifiers:
  - Internal crate/module prefix: `koushi-*`
  - Tauri bundle identifier: `chat.koushi.desktop`
  - npm/Cargo package name: `koushi-desktop`
  - keychain / file credential-store service name: `koushi-desktop`
  - Matrix global account-data key for local user aliases:
    `app.koushi.local_aliases`
- No migration from old Matrix Desktop/Kagome identifiers is required, because
  there is no supported persisted user data to preserve. New storage, QA env
  vars, keychain entries, account-data keys, and internal app event schemes use
  Koushi identifiers. Renaming them again requires an explicit migration plan
  and user approval.

## Concurrent Work And Merge-Conflict Avoidance

### Test Placement

- Integration-style tests live in per-feature `crates/<crate>/tests/<feature>.rs`
  files, never in new additions to monolithic test files; inline `cfg(test)`
  modules have a 200-line hard ceiling. Full rules: engineering rules "Test
  Placement".

### Shared Hot Files

- The main agent owns integration of the shared hot files listed in engineering
  rules "Concurrent Work Details"; subagents may read them but must not append to
  them without main-agent coordination.

### Parallel Implementation Protocol

- The main agent fixes shared surface design before parallelizing, never runs two
  agents on one hot file, and integrates shared enums, reducers, wire types, and
  generated artifacts itself; subagent output is a draft. Parallel branches land
  through an integration worktree. Details: engineering rules "Concurrent Work
  Details".

### Worktree And Build Artifact Cleanup

- Remove merged temporary worktrees promptly together with their unshared build
  intermediates, never delete a shared `CARGO_TARGET_DIR`, and verify with
  `git worktree list`. Details: engineering rules "Concurrent Work Details".

## Review And Audit

- **The main agent owns review and acceptance.** It reads the finished diff,
  including new files, against the applicable canon and verification evidence.
  Delegated output is a draft. Routine changes use self-review and
  deterministic checks; delegation alone imposes no separate design-document or
  two-review ceremony.
- **Independent review follows risk, not model branding.** Changes to crypto,
  authorization, unsafe/FFI contracts, consequential concurrency, or major
  cross-layer boundaries require read-only independent review of the design
  before implementation and of the integrated diff afterwards; fix blocking
  findings and record the verdicts. Use an available subagent review tool, a
  different model family preferred. Explicit user or higher-priority agent
  review requirements still apply. If required review is unavailable, report the
  blocker instead of claiming readiness.
- **Review priorities, in order:** consistency with repository rules and canon;
  consistency with Rust/Tauri best practice and the existing codebase; security
  and privacy (secret leakage, unsafe code, untrusted input, cross-boundary
  exposure); correctness of state-machine, command/event, and DTO contracts.
- **Rule gaps become rule-update proposals.** A problem caused or enabled by a
  gap, ambiguity, or missing rule yields a proposed amendment to this file or the
  engineering rules, not only a code patch; the main agent adopts, escalates, or
  defers it.
- **Findings are implementation tasks.** Blocking issues are addressed and the
  relevant gates re-run before landing on `main`.
- **Self-review is required regardless of author:** run the applicable gates and
  read the finished diff before claiming completion; independent review does not
  replace it. Escalate unresolved cross-boundary decisions to the user before
  landing. Audit depth is proportional to risk: a module-local patch may need a
  quick diff check, while a change across shared enums, reducers, command/event
  variants, Tauri DTOs, TypeScript wire, and generated contracts needs a thorough
  cross-boundary audit.
- **Review material stays private-data-free.** Subagent prompts, review notes,
  PR bodies, and issue comments use only synthetic data, never real credentials,
  room/event IDs, message bodies, raw SDK errors, or local paths.

## GitHub Issue Language

Write GitHub issue titles, descriptions, and agent-authored comments in English,
including research findings and follow-up updates, unless the user explicitly
requests another language. Preserve exact UI labels, quotations, and other
source text in their original language when needed; explain them in English.
This policy does not change the language used to converse with the user.

## GitHub Issue Closure

- Close an issue once its only remaining acceptance items are manual or
  real-device checks (native GUI, live homeserver, throttled or offline
  desktop). Manual inspection is confirmation, not correctness evidence, so it
  must not keep an issue open. The closing comment lists the pending manual
  checks and the headless evidence (the RED-then-GREEN tests) that the fix
  rests on. A defect found later during manual checking gets a new issue.
- Keep an issue open while any automatable test (headless, integration,
  renderer), unfixed defect, or open decision remains. When only a user
  policy decision remains, move it to its own issue and close the original.
- Put closing keywords (`Closes`, `Fixes`, `Resolves`) only in the PR body
  of a PR that meets the criteria above; commit messages use `Refs #N`, since
  a keyword in a merged commit closes the issue regardless of the PR body.
  GitHub treats every tense (close/closes/closed, fix/fixes/fixed,
  resolve/resolves/resolved) followed by `#N` anywhere in a merged PR
  description or commit message as a closing keyword, so describe past events
  without that pattern (for example "#1061 was auto-closed").

## Documentation And Work Records

- Before opening or updating a PR that changes user-visible behavior, check the
  matching user-guide pages against the implementation (menu/category paths, UI
  labels, prerequisites, outcomes, limitations) and update them in the same PR,
  or record why no help change is needed. Settings moves also update
  `docs/help/settings.md`. Check the generated llms.txt and links; mechanical
  link checks do not replace this review.
- Dated implementation plans are subordinate to the normative docs: when a
  discovery changes architecture or rules, amend the canon first, then sync or
  supersede the plan.
- When an umbrella child issue completes, record its discoveries where they
  belong: durable architecture/rule changes in `docs/architecture/`, this file,
  or the engineering rules; operational setup/failure notes in `docs/agents/`;
  QA scenario contracts in `docs/agents/qa-lanes.md` or `docs/qa/`. Closing an
  issue without syncing the learned rule is a process defect.
- Nontrivial agent-driven work should leave a short plan, worklog, review
  record, or changelog entry naming the canon consulted, files changed, and
  verification run; this is required for state-machine, security, SDK fork surface, QA gate,
  or release gate changes.
- Operational setup and failure notes go in the matching `docs/agents/` topic,
  not `AGENTS.md` itself, until they become durable rules;
  `scripts/check-agents-docs.mjs` enforces that routing.

## Licensing

- Code or design ported from Element, Seshat, Matrix Rust SDK, FluffyChat, or
  related upstream projects preserves applicable license and copyright notices.
- Prefer upstreamable changes for `matrix-sdk-search` and the vendored Matrix
  Rust SDK; keep local patches small, documented, and suitable for upstream
  feedback.
