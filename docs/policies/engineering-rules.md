# Engineering Rules

Status: normative detailed policy. This document extends the root durable rules
in [REPOSITORY_RULES.md](../../REPOSITORY_RULES.md) with concrete policy for
secrets, logging, QA automation, async/runtime behavior, GUI automation, and
build gates. AGENTS.md remains the operational how-to (permissions, install
caveats, recovery steps); durable rules discovered there are promoted to
REPOSITORY_RULES.md or this document.

Last amended: 2026-06-15.

## Secrets and Private Data

Never log, print, commit, or store in fixtures:

- access tokens, passwords, recovery keys or recovery codes
- SDK store keys, search index keys, local unlock secrets
- raw request/response bodies
- real account private data; real room names or real discussion content in
  docs, tests, or mocks

Allowed only in debug/test contexts: synthetic local QA credentials, local
homeserver URLs, synthetic room/event IDs. Allowed in UI state: user ID,
device ID, room ID, event ID, visible message body, attachment filename.

Rules:

1. Secret-bearing types must use zeroizing wrappers with redacted `Debug`
   (`finish_non_exhaustive()` style). This includes command payloads:
   login requests redact username/password/device name; recovery requests
   redact recovery material; send/edit redact bodies in `Debug` and errors.
2. Release builds must reject environment-variable credential injection and
   the file-based credential store. The gate is compile-time (debug/test
   only) and CI must verify release builds ignore these paths.
3. QA credentials enter processes via FIFO (`MATRIX_DESKTOP_QA_LOGIN_PIPE`)
   or the gated file credential store — never via argv, never typed by
   coordinates, never echoed to a terminal, never in screenshots or logs.
4. Do not pass the parent shell environment wholesale into QA child
   processes. Filter out secret-like variables (API keys, tokens,
   passwords) before spawning.
5. Do not store post-login real-account screenshots; they can contain room
   names, Matrix IDs, message bodies, attachment names. Use
   private-data-free QA window-title tokens. `--allow-private-screenshots`
   is restricted to explicitly approved test accounts and ignored artifact
   paths.
6. QA profile names must be synthetic and non-secret. Profile data lives
   under ignored `.local-secrets/qa-profiles/<name>/data`.
7. A secret scan gate runs before commits **and** in CI (pre-commit hooks
   can be bypassed). It excludes `vendor/`, `.local-secrets/`, and
   generated artifacts.
8. An unexpected macOS Keychain prompt during unattended QA is an
   automation failure, not something to click through. Fix the run's
   environment (`MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE=1`,
   `MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR`) instead.
9. OS notifications, badge labels, and QA window-title tokens are
   private-data-minimized surfaces. By default they may include only a safe
   room display label, notification kind (`mention`, `dm`, `message`), and
   aggregate unread/highlight counts. They must not include message bodies,
   sender identifiers, room IDs, event IDs, transaction IDs, raw SDK errors,
   or secrets.
10. Device-local settings are non-secret product state, but they are still a
   privacy boundary. Settings files may contain only typed preferences such as
   locale, theme, font/emoji choice, keyboard behavior, and notification
   policy. They must not contain Matrix credentials, tokens, recovery material,
   local unlock secrets, SDK/search keys, raw Matrix session JSON,
   room/event/user IDs, message bodies, attachment filenames, search queries,
   or raw SDK errors.
   Derived display profiles for these settings, including
   `LocaleDisplayProfile` and `TypographyDisplayProfile`, are also
   non-secret profile data only. They may carry platform/capability and
   asset-status tokens, but not account identifiers, content, local paths, raw
   errors, or credentials.
11. E2EE trust diagnostics are kind-only. Verification, cross-signing,
   key-backup, and identity-reset commands/events may expose structured state to
   the UI, but normal `Debug`, QA logs, and window-title tokens must redact
   account keys, verification target user/device IDs, backup versions, raw SDK
   errors, identity-reset auth details beyond UIAA/OAuth/unknown, and all key
   material.
12. Media/file diagnostics are metadata-minimized. `CoreCommand` may carry
   filename, caption, mimetype, dimensions, and bytes when sending media, and
   `TimelineItem.media` may expose safe render metadata. Normal `Debug`, QA
   logs, errors, window-title tokens, and docs examples must not expose
   filenames, captions, bytes, MXC URIs from real accounts, encrypted media
   keys/hashes, room IDs, event IDs, or raw SDK errors. Download effects emit
   byte counts or app-owned handles only; downloaded bytes stay in Rust-owned
   effects or platform ports.
13. Profile/avatar diagnostics are metadata-minimized. Display names and avatar
   bytes may cross only the typed command or snapshot boundaries needed for the
   UI; normal `Debug`, QA logs, errors, window-title tokens, and issue evidence
   must not expose real display names, avatar MXC URIs, avatar bytes, local
   thumbnail paths, encrypted media keys/hashes, or raw SDK errors. React must
   render avatar images only from Rust/platform-owned ready source URLs and must
   fall back to generated initials for MXC, loading, or failed thumbnail states.

## Logging and Diagnostics

1. Diagnostics are structured and redacted
   (`core.sync.failed kind=http` style). Structured fields are enums/kinds;
   free-form string fields are prohibited because they eventually carry
   content.
2. Raw SDK errors may be printed only behind an explicit debug/test
   diagnostic switch. They must never reach `AppState`, committed logs,
   normal test fixtures, or release diagnostics.
3. QA asserts on `CoreEvent` and `AppStateSnapshot`, never on log output.
4. Real-account and real-homeserver QA output is tokenized before it becomes an
   artifact. Captured logs must not contain raw Matrix IDs, event IDs,
   transaction IDs, user IDs, message bodies, search queries, local paths, or raw
   SDK errors. Producers should avoid formatting those values; wrappers must not
   write unredacted stdout/stderr and only then discover a leak.

## Async and Runtime

1. No fixed sleeps in QA or product code waiting for Matrix effects — wait
   on events with timeouts.
2. Store-backed Matrix SDK clients must be dropped while a Tokio runtime
   context is entered; otherwise `deadpool-runtime` panics with
   "there is no reactor running".
3. Every spawned background task and subscription has an owner responsible
   for cancelling it (unsubscribe, account shutdown, app shutdown). No
   unbounded maps of live subscriptions.
4. Timeline scrollback is a split contract: core emits diffs and pagination
   state; React owns DOM anchoring. Product code must not issue automatic
   pagination loops before the previous diff has rendered and anchor
   restoration has completed.
5. QA runners must clean up their full process group on failure or
   interruption. Verify `lsof -nP -iTCP:5173 -sTCP:LISTEN` is empty before
   retrying a GUI run; a stale Vite/`tauri dev` process breaks the next run.
6. QA binaries must attempt logout cleanup after any post-login failure
   unless `--keep-session` was explicitly requested; otherwise failed runs
   leave live devices on the homeserver.
7. Avoid repeated destructive real-account login cycles while debugging
   automation; reuse the running session and restart only when the script
   or Tauri capability changes require it.
8. State-critical actor actions are reliable messages, not lossy hints. Do not
   ignore failed reducer-action sends for transitions that set or clear pending
   user-visible state. Await the send, retry through the owner, or emit a
   correlated operation failure that leaves no stuck pending state.
9. If a reducer returns an `AppEffect` that matters in production, the
   production runtime executes it or the behavior is redesigned as an explicit
   `CoreCommand`/actor command. Discarding such effects is allowed only for
   fixture/demo effects that are documented as non-production.

## GUI Automation

GUI automation is a thin smoke layer, never the primary correctness gate.

0. UI behavior is verified headless by default: frontend tests run in a
   headless browser with mocked Tauri IPC and fake `CoreEvent` streams
   (QA Model layer 4). The real Tauri app is launched only for the minimal
   native-integration smoke, and on macOS only attended — unattended agent
   sessions must not launch the GUI app (it opens real windows, reads the
   OS keychain, and surfaces crash dialogs on the user's desktop).
   The repository's canonical headless DOM gate is currently
   `npm --prefix apps/desktop run test:ui-headless` using Playwright against
   the Vite harness. `@wdio/tauri-service` browser mode may be adopted only
   after a spike proves the installed package can run the frontend in a
   normal browser without a Tauri binary, native driver, native window, or
   OS keychain access.
   Real-Tauri GUI automation by agents is allowed only under a virtual
   display (Linux Xvfb + `tauri-driver`; not available on macOS). The goal
   is that agents carry GUI design and testing as far as headless/virtual
   harnesses allow; only macOS-specific native behavior remains attended.
   i18n GUI work must also prove the Rust-resolved `LocaleDisplayProfile`
   reaches the DOM root (`lang`, `dir`, catalog, pseudo mode), remote/user text
   keeps `dir="auto"`, pseudo/CJK/RTL samples do not overflow the shell, and
   raw user-facing strings/logical-CSS contracts are covered by headless tests.
1. Never drive login or any credential entry by fixed window-relative
   coordinates (a 2026-06-12 run typed a password into the username field).
   Use the FIFO credential path.
2. Never use `Cmd+Q` to stop the app from automation; focus slips can send
   it to the controlling agent. Use the script's process-group cleanup.
3. Resolve processes as `first process whose name is <variable>` in
   AppleScript; check both the dev process name (`matrix-desktop-app`) and
   the product title (`matrix-desktop`).
4. First-run GUI smoke sets `MATRIX_DESKTOP_SKIP_SAVED_SESSIONS=1`;
   real-login smoke additionally sets
   `MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE=1`.
5. Keep the strict `timeline_items > 0` release signal; use
   `--allow-empty-timeline` only for sparse test accounts validating
   login/room-list/panel automation.

Operational setup (Accessibility/Automation/Screen Recording permissions,
PTY handling, prompt line order) is documented in `AGENTS.md`.

## Build, Dependencies, QA Gates

0. New Matrix behavior is implemented and verified headless-first: it lands
   in `matrix-desktop-core`, is exercised via `CoreCommand`/`CoreEvent`
   against local Conduit/Tuwunel homeserver QA, and only then is wired into
   Tauri/React. GUI-first Matrix behavior is prohibited.
1. The vendored `matrix-rust-sdk` is consumed via path/`[patch]`
   dependencies, preserving upstream structure for upstreaming patches.
   Direct ports from Element X code preserve upstream license and copyright
   notices.
   Patches to the vendored SDK are limited to what is indispensable: a
   change is allowed only when the need cannot be met through the SDK's
   public API or a wrapper on our side. Each patch must be minimal
   (prefer additive accessors over behavioral changes), recorded in
   `docs/upstream/matrix-rust-sdk-feedback.md` with rationale and
   upstreaming intent, and reviewed at phase exit. In this repo the actual
   deltas live on the `github.com/shinaoka/matrix-rust-sdk-work`
   submodule branch (`shinaoka/search-ngram`), and local comments should
   point at the patch surface.
   Convenience patches are rejected; every patch increases the cost of
   tracking upstream.
2. Local homeserver toolchain caveats (Conduit/Tuwunel install flags such as
   `RUMA_UNSTABLE_EXHAUSTIVE_TYPES=1`, macOS `--no-default-features`) are
   tracked in `AGENTS.md` and the QA scripts, not hand-run.
3. Required local gates before merge: crate tests (`matrix-desktop-state`,
   `-auth`, `-core`), frontend tests + typecheck, and
   `qa:headless-local -- --server=both`.
4. Real homeserver QA is a release/preflight gate (network + approved
   credentials), not an every-CI gate.
   It is also required before GUI-level confidence claims and after changes
   that affect login, recovery, sync, encrypted restore, search, room cleanup,
   or logout.
5. Production Tauri paths must not execute fixture-backend behavior;
   `matrix-desktop-backend` is dev/demo only.
6. Core crates stay platform-portable (a future browser/wasm target must not
   be precluded): no Tauri/OS/filesystem types in `CoreCommand`/`CoreEvent`/
   `AppStateSnapshot`; task spawn and timers via executor abstractions, not
   direct `tokio::spawn`/`tokio::time` in actor logic; `keyring`, paths, and
   store config only behind `StoreActor`/adapter ports;
   `matrix-desktop-state` and `matrix-desktop-search` must compile for
   `wasm32-unknown-unknown`. See Platform Portability in
   `docs/architecture/overview.md`.

## Documentation

1. `REPOSITORY_RULES.md` is the root durable rule book for this repository.
   This document is the detailed policy extension for the domains it covers.
2. `docs/architecture/overview.md` is the long-term blueprint. Dated specs
   and plans implement it; when implementation reveals a design problem,
   amend the overview first.
3. Durable rules discovered during operations are promoted from `AGENTS.md`
   into `REPOSITORY_RULES.md` or this document; AGENTS.md keeps the
   troubleshooting detail.
4. Docs, examples, and fixtures use synthetic data only (see Secrets rules).
5. State-machine diagrams are normative. Every reducer state machine in
   `reduce(AppState, AppAction)` — its states, transitions, and guards — is
   documented as a Mermaid `stateDiagram-v2` in
   `docs/architecture/state-machine.md`. A change to a reducer state machine
   must update the matching diagram and its guard notes in the same change; a
   transition present in code but not in the diagram (or the reverse) is a
   defect. Phase-exit docs-sync verifies diagram↔reducer agreement. Design new
   state transitions as explicit guarded state machines (events distinct from
   states, invalid/stale inputs rejected), not ad-hoc field assignments.
6. For umbrella issue work, each child issue completion must record
   implementation discoveries in the right place: durable architecture/rule
   changes in `docs/architecture/`, `REPOSITORY_RULES.md`, or this document;
   operational setup/failure notes in `AGENTS.md`; and QA scenario contracts in
   `docs/qa/`. Closing an issue without syncing the learned rule is a process
   defect.
