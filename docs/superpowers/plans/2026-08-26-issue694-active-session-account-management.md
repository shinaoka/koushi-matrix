# Issue #694 Priority 2 — Active-session account management

## Outcome

The active Matrix session owns one optional, URL-safe account-management
destination. User Settings renders **Manage account & devices** only when that
destination is available and delegates remote account/device management to the
server. Koushi no longer renders its remote-device list, rename, or sign-out UI.
Login-screen discovery owns login flows and registration only.

This follows the Element X split recorded in #694: the client retains current
session verification/diagnostics and local saved-account switching, while the
server-provided account-management UI owns remote sessions/devices.

## Contract

- `AppState.account_management_url: Option<AccountManagementUrl>` is the sole
  UI-facing destination and is scoped to the exact active `SessionInfo`. The
  transparent string newtype redacts its Debug representation. It is distinct
  from the existing `account_management` / `account_management_capabilities`
  UIAA state used only by local password change and account deactivation.
- Login discovery no longer carries an account-management URL: remove that
  field from `DelegatedAuthLinks` and its snapshot mirrors. It may continue to
  expose the delegated registration URL needed before authentication. The
  active-session SDK resolver reuses the URL parser, not the login DTO field.
- `AccountActor` starts one owned discovery task after a session is promoted.
  Authentication lock, logout, account switch, trust quarantine, replacement,
  and shutdown invalidate and abort it. Completion carries the originating
  `SessionInfo`; the reducer
  accepts it only while that exact session is current, so stale completion cannot
  populate another account.
- OAuth sessions call public upstream matrix-rust-sdk
  `OAuth::server_metadata()` and
  `AuthorizationServerMetadata::account_management_url_with_action()` with the
  devices-list action.
- Non-OAuth sessions fetch only `/.well-known/matrix/client` from the active
  session homeserver, accept finalized `m.authentication` and legacy
  `org.matrix.msc2965.authentication`, and accept only HTTP(S) account URLs.
- Network, status, parse, metadata, or URL validation failure returns `None`.
  It never blocks login, restore, admission, or normal runtime.
- React performs no discovery, URL construction, account identity fencing, or
  retry. It opens only the Rust-projected URL.
- Current-session diagnostics/device-name repair, saved-account switching,
  verification, secure backup, provisional current-device cleanup, password and
  deactivation UIAA remain unchanged.

## Verify-first matrix

Before production changes, record RED evidence for:

1. SDK active-session resolver:
   - OAuth uses devices-list action metadata;
   - password well-known accepts finalized and matrix.org legacy keys;
   - missing, malformed, non-200, and non-HTTP(S) values return `None`.
2. Reducer ownership:
   - exact active-session completion installs the URL;
   - switched/logged-out/stale-session completion is ignored;
   - session cleanup invalidates the URL.
3. Actor lifecycle:
   - restored/promoted password and OAuth sessions start discovery without login
     discovery;
   - replacement, authentication lock, trust quarantine, logout, and shutdown
     abort/invalidate an in-flight completion.
4. GUI and QA mirrors:
   - no local remote-device list/rename/sign-out controls or IPC invocations
     remain, and the old profile-settings browser scenario is retired/replaced;
   - **Manage account & devices** is absent without a URL and opens the active
     session URL when present;
   - login discovery account URL cannot drive the action;
   - E2EE send diagnostics no longer report remote-inventory counters and obtain
     only current-device verification from `current_session_status`;
   - the frontend state golden contains a non-null safe destination so the new
     snapshot shape is exercised.

## RED evidence

Recorded before production changes:

- `cargo test -p koushi-state --test active_session_account_management` failed
  because `AppState.account_management_url` and
  `ActiveSessionAccountManagementUrlResolved` did not exist (12 compile errors).
- `cargo test -p koushi-sdk --lib active_session_account_management_tests`
  failed because the active-session resolver did not exist.
- `cargo test -p koushi-core --lib
  promoted_restored_session_starts_active_account_management_discovery` failed
  because no actor-owned destination completion action existed.
- focused User Settings/App Vitest failed four assertions: the unavailable
  disabled action and local device/session controls still rendered, and App
  still invoked login discovery after restore.

## Implementation sequence

1. Add the Rust state/action/reducer field and exact-session fence; keep the
   matching install/clear/guard transitions in `docs/architecture/state-machine.md`
   synchronized; update DTO, state-delta, TypeScript, browser-fake, diagnostic,
   and non-null golden mirrors.
2. Add the SDK active-session resolver using only public vendored APIs.
3. Add the owned AccountActor task and teardown invalidation.
4. Move every desktop account-management read to the active-session field and
   delete the React login-rediscovery workaround.
5. Delete the remote-device inventory/ordinal, rename, delete, command, DTO,
   and UI state machine end to end; keep current-device provisional cleanup and
   password/deactivation UIAA paths intact.
6. Run focused GREEN, full relevant gates, rendered browser-headless proof,
   preflight self-review, and independent cross-model review. Fix and re-run
   affected gates before PR creation.
7. Rebase on current `origin/main`, push a reviewable PR referencing #694, wait
   for every required check, merge only while current/green, and verify the
   merged tree on `origin/main` with no vendor SDK diff.

## Final local evidence

After all accepted-review fixes:

- `cargo test --workspace`: 2584 passed / 13 ignored;
- `cargo check --workspace`: passed;
- Core QA binary tests: 133 passed;
- frontend Vitest: 1497 passed; typecheck, lint/IME/agent-docs, and production
  build passed;
- full browser-headless Playwright: 263 passed;
- rustfmt check, `git diff --check`, SDK submodule guard, and agent-doc checks
  passed;
- SDK gitlink remains `bc90003576d913ab21670c26e24c3c9b45fd15d1`
  and its worktree is clean.

## Review record

Pre-implementation reviewer selected by the user: `reviewer-flash`
(DeepSeek family, read-only, high thinking).

- Round 1: `Not-correct-to-merge`. The reviewer required explicit auth-lock,
  quarantine, and shutdown invalidation tests; state-machine synchronization in
  the sequence; removal coverage for E2EE diagnostics/browser IPC; and a
  non-null golden mirror. These findings are incorporated above.
- Round 2: `Correct-to-merge`. All Round 1 findings were verified fixed. The
  reviewer left two non-blocking precision notes; the diagram now names generic
  session cleanup and the plan explicitly removes the login DTO field while
  reusing only its parser.

Post-implementation self-review traced SDK → actor task → reducer → delta/DTO →
React and every teardown path. It found and fixed two ownership issues before
independent review: the destination now uses a transparent Debug-redacted
`AccountManagementUrl` newtype end to end, and a successful completion awaits
its owned task handle instead of dropping/detaching it.

Post-implementation independent review (`reviewer-flash`, read-only, high):

- two initial broad review attempts timed out without verdict and were not
  accepted;
- Review A (SDK/actor/reducer lifecycle): `Correct-to-merge`; three minor test
  gaps were subsequently closed (OAuth metadata failure, well-known network
  failure, and direct `Some → None` reducer settlement);
- Review B1-removal: `Correct-to-merge`; all remote-device-manager symbols were
  absent except the explicitly preserved current-device repair, provisional
  cleanup, and QA fixture paths;
- Review B1-wire: `Correct-to-merge`; schema v5, full/delta nested-null,
  non-null golden, generated event artifact, UIAA/preservation, and SDK gitlink
  were verified;
- Review B2 Round 1: `Not-correct-to-merge`; User Settings gated the external
  action on truthiness rather than URL safety;
- Review B2 Round 2: `Correct-to-merge` after AccountManagementSection reused
  `toExternalHttpUrl` and unit/Playwright unsafe-URL proofs were added.
