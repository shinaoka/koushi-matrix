# Desktop Foundation Before Login

Status: historical foundation. The current production runtime architecture is
documented in [overview.md](./overview.md).

Date: 2026-06-11

This document captures the repository shape from the foundation stage, just
before real Matrix login moved into the current core runtime.

## Current Layers

```text
apps/desktop-shell
  static Slack-like shell over fake ready-session data

apps/desktop
  Tauri v2 + React shell over the same snapshot DTOs
        |
        v
crates/koushi-backend
  fake effect runner around reducer/search/key contracts
        |
        +--> crates/koushi-sdk
        +--> crates/koushi-state
        +--> crates/koushi-search
        +--> crates/koushi-key
```

`koushi-backend` executed the same loop the real backend would use in
that foundation stage:

```text
AppAction -> reduce(AppState) -> AppEffect -> backend effect runner -> follow-up AppAction
```

The effect runner faked session restore, sync start, timeline subscription,
thread subscription, sending synthetic local text, and search. Login discovery
and password login were switchable in that phase: tests and the browser fallback
could use deterministic fixture flows, while the Tauri runtime could call the
homeserver and perform password login through Matrix Rust SDK. That was the
foundation step; current production behavior now lives in `overview.md` and
uses `CoreCommand` / `CoreEvent`.

`koushi-sdk` owns Matrix authentication discovery and password login.
It normalizes homeserver URLs to HTTPS by default, permits plain HTTP only for
localhost or loopback development servers, builds `GET /_matrix/client/v3/login`,
parses the response into app DTOs, calls Matrix Rust SDK for password login, and
keeps raw homeserver response bodies out of long-lived state. The Tauri command
runner executes SDK password login outside the backend mutex and then returns
only the success or failure action to the state machine.

## Login Boundary

At that stage, the planned real-login work was expected to attach at these
points:

1. `AppEffect::DiscoverLogin` could query `GET /_matrix/client/v3/login` on
   the configured homeserver and record supported flows such as
   `m.login.password`, `m.login.sso`, or `m.login.token`.
2. The UI enabled the password path only when discovery reported
   `m.login.password`; SSO/OIDC-capable homeservers can branch into a browser
   flow from the same snapshot state.
3. `AppEffect::Login` marked the point where password login left the state
   machine. In the Tauri runtime, `submit_login` released the backend lock,
   created a `matrix_sdk::Client` using the configured homeserver, and called
   `matrix_auth().login_username(...)` on Tauri's blocking task pool. The login
   request carried homeserver, login identifier, password, and device display
   name. The password was an in-memory redacted secret and did not enter
   `AppState`, frontend snapshots, debug output, logs, or persisted stores.
4. Successful login dispatched `AppAction::LoginSucceeded(SessionInfo)` and
   kept the SDK client in memory for the current process.
5. `koushi-sdk` could extract a redacted
   `PersistableMatrixSession` from the SDK client and restore a fresh SDK client
   from that payload. The serialized JSON contains access/refresh tokens and is
   therefore a secret; it must go only to an approved secure store.
6. `koushi-key` had a redacted `StoredMatrixSession` wrapper and
   account-name separation for Matrix session JSON in the OS credential store.
   The next persistence step must connect `AppEffect::PersistSession` to that
   store and add a restore pointer/index for finding the last account/device at
   app startup.
7. `koushi-key` loaded or created the local unlock secret through the OS
   credential store.
8. The SDK store key and search index key were derived from that local unlock
   secret.
9. `AppEffect::StartSync` still used fake data. The planned next step was to
   start SDK sync, room-list services, timeline subscriptions, and search
   indexing.

The default homeserver remained `https://matrix.org`, but
`FakeDesktopBackendConfig` kept homeserver configurable. The real login UI
should expose the same setting before submitting credentials. Users do not
need to type `https://`; bare homeserver input such as `matrix.org` is
normalized to HTTPS. Explicit ports such as `matrix.example.org:8448` are
allowed. Plain `http://` remains restricted to localhost or loopback
development servers.

## Pre-Login Shell

The app now has an explicit first-run path before real Matrix login:

1. `AppEffect::RestoreSession` may resolve to `AppAction::RestoreSessionNotFound`.
2. The reducer enters `SessionState::SignedOut` without recording an error.
3. The React shell rendered a homeserver-configurable sign-in form instead of
   the Slack-like ready surface.
4. `discover_login_methods` dispatches `AppAction::LoginDiscoveryRequested`,
   which emits `AppEffect::DiscoverLogin`.
5. The browser fallback returned synthetic password and SSO flows so the UI
   could exercise the same branching contract without external network
   dependency. The Tauri runtime used HTTP discovery by default.
6. `submit_login` dispatches `AppAction::LoginSubmitted`, which emits
   `AppEffect::Login`.
7. The browser fallback intentionally turned that effect into
   `AppAction::LoginFailed` with a non-secret message. The Tauri runtime
   deferred that effect to the native command runner, which used Matrix Rust
   SDK password login outside the backend mutex.

Recovery key or security phrase input is not part of Matrix login. It belongs
after login, when the client needs to restore encrypted room-key backup or
cross-signing secrets. That recovery input must have the same secret-handling
rules as passwords and must not be stored in React state longer than the active
recovery step requires.

The desktop shell modeled that boundary with a post-login `needsRecovery`
session state. While in that state, sync, room navigation, timeline, thread,
and search effects stayed blocked because the session was not `ready`. The
frontend rendered a dedicated recovery screen rather than reusing the login
form. The recovery key/security phrase was read from an uncontrolled input,
submitted to the backend, and cleared immediately after submission.

The Matrix Rust SDK exposes this as
`client.encryption().recovery().recover(...)`. The present Tauri command had
the state/effect boundary for that call, but the actual SDK recovery
invocation was still a follow-up task.

Open the browser shell in first-run mode with:

```bash
http://127.0.0.1:5173/?session=signed-out
```

Open the browser shell directly at the recovery step with:

```bash
http://127.0.0.1:5173/?session=recovery
```

Open the Tauri shell in first-run mode with:

```bash
MATRIX_DESKTOP_RESTORE_SESSION=0 npm run tauri dev
```

Before typing live credentials into the native shell, the SDK password-login path
can be smoke-tested from a terminal without storing secrets:

```bash
cargo run -p koushi-sdk --features smoke --bin password-login-smoke
cargo run -p koushi-sdk --features smoke --bin password-login-smoke -- --real-account-qa
```

The smoke command prompts interactively, hides the password, prints no access
token, and logs out by default after a successful login. The `--real-account-qa`
variant also verifies in-memory persistable session export/import, SDK session
restore, one SDK sync, private-data-free room-list counts, and private-data-free
selected-room timeline item counts after timeline backfill. Use `-- --keep-session`
only when deliberately leaving the Matrix device/session alive for follow-up
manual testing.

SDK sync, token persistence, and restore should replace only effect handlers.
The UI should continue to read `SessionState` and `AppError` through the same
snapshot DTOs.

## Search Boundary

`koushi-backend` passes broad candidate events into `koushi-search`. This mirrors the intended `matrix-sdk-search` contract:

- the SDK search layer owns encrypted Tantivy indexes and ngram candidate retrieval;
- `koushi-search` owns exact verification against the resolved visible event body or attachment filename;
- edits are resolved before highlighting;
- redacted events are removed before verification;
- false positives are dropped when no exact span exists.

Search-result snippets and highlights are produced only after verification. The current DTO uses half-open UTF-16 offsets.

## Desktop Shell

`apps/desktop` is the Tauri v2 + React shell. It renders snapshot DTOs through a browser fallback API outside Tauri and Tauri commands inside the native shell.

`apps/desktop-shell` remains a zero-dependency local reference shell. Both render:

- Space rail;
- Space-filtered room list;
- global DM section;
- selected timeline;
- right thread pane on wide viewports;
- exact-highlighted search results;
- attachment filename search results.

Open it directly as a static file or serve it with a local HTTP server:

```bash
cd apps/desktop-shell
python3 -m http.server 4173 --bind 127.0.0.1
```

Then open `http://127.0.0.1:4173/`.

For the React/Tauri shell:

```bash
cd apps/desktop
npm install
npm run dev
```

Then open `http://127.0.0.1:5173/`.

## Not Done Yet

- Release bundling/signing.
- SDK sync, room-list, and timeline service wiring.
- Persisted session restore.
- Persistent encrypted search indexes.
- E2EE store initialization and recovery UI.
- Video chat.
