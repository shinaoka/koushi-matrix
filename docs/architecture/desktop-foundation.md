# Desktop Foundation Before Login

Date: 2026-06-11

This repository is now structured up to the point immediately before real Matrix login.

## Current Layers

```text
apps/desktop-shell
  static Slack-like shell over fake ready-session data

apps/desktop
  Tauri v2 + React shell over the same snapshot DTOs
        |
        v
crates/matrix-desktop-backend
  fake effect runner around reducer/search/key contracts
        |
        +--> crates/matrix-desktop-auth
        +--> crates/matrix-desktop-state
        +--> crates/matrix-desktop-search
        +--> crates/matrix-desktop-key
```

`matrix-desktop-backend` executes the same loop the real backend will use:

```text
AppAction -> reduce(AppState) -> AppEffect -> backend effect runner -> follow-up AppAction
```

The effect runner still fakes session restore, sync start, timeline subscription,
thread subscription, sending synthetic local text, and search. Login discovery is
now switchable: tests and the browser fallback can use deterministic fixture
flows, while the Tauri runtime can call the homeserver over HTTP. Real Matrix
integration should replace the remaining fake handlers with a Matrix SDK runner
without moving state transitions into the UI.

`matrix-desktop-auth` owns Matrix authentication discovery. It normalizes
homeserver URLs to HTTPS by default, permits plain HTTP only for localhost or
loopback development servers, builds `GET /_matrix/client/v3/login`, parses the
response into app DTOs, and keeps raw homeserver response bodies out of
long-lived state.

## Login Boundary

The next real-login step should attach at these points:

1. `AppEffect::DiscoverLogin` can now query `GET /_matrix/client/v3/login` on
   the configured homeserver and record supported flows such as
   `m.login.password`, `m.login.sso`, or `m.login.token`.
2. The UI enables the password path only when discovery reports
   `m.login.password`; SSO/OIDC-capable homeservers can branch into a browser
   flow from the same snapshot state.
3. `AppEffect::Login` creates a `matrix_sdk::Client` using the configured homeserver.
   The login request carries homeserver, login identifier, password, and device
   display name. The password is an in-memory redacted secret and must not enter
   `AppState`, frontend snapshots, debug output, logs, or persisted stores.
4. Successful login dispatches `AppAction::LoginSucceeded(SessionInfo)`.
5. The backend creates a `SessionKeyId` from homeserver, user id, and device id.
6. `matrix-desktop-key` loads or creates the local unlock secret through the OS credential store.
7. The SDK store key and search index key are derived from that local unlock secret.
8. `AppEffect::StartSync` starts SDK sync, room-list services, timeline subscriptions, and search indexing.

The default homeserver remains `https://matrix.org`, but `FakeDesktopBackendConfig` already keeps homeserver configurable. The real login UI should expose the same setting before submitting credentials.

## Pre-Login Shell

The app now has an explicit first-run path before real Matrix login:

1. `AppEffect::RestoreSession` may resolve to `AppAction::RestoreSessionNotFound`.
2. The reducer enters `SessionState::SignedOut` without recording an error.
3. The React shell renders a homeserver-configurable sign-in form instead of the Slack-like ready surface.
4. `discover_login_methods` dispatches `AppAction::LoginDiscoveryRequested`,
   which emits `AppEffect::DiscoverLogin`.
5. The browser fallback returns synthetic password and SSO flows so the UI can
   exercise the same branching contract without external network dependency.
   The Tauri runtime uses HTTP discovery by default.
6. `submit_login` dispatches `AppAction::LoginSubmitted`, which emits `AppEffect::Login`.
7. The fake backend intentionally turns that effect into `AppAction::LoginFailed` with a non-secret message because real Matrix login is not wired yet.

Recovery key or security phrase input is not part of Matrix login. It belongs
after login, when the client needs to restore encrypted room-key backup or
cross-signing secrets. That recovery input must have the same secret-handling
rules as passwords and must not be stored in React state longer than the active
recovery step requires.

Open the browser shell in first-run mode with:

```bash
http://127.0.0.1:5173/?session=signed-out
```

Open the Tauri shell in first-run mode with:

```bash
MATRIX_DESKTOP_RESTORE_SESSION=0 npm run tauri dev
```

Real SDK login should replace only the `AppEffect::Login` effect handler. The UI should continue to read `SessionState` and `AppError` through the same snapshot DTOs.

## Search Boundary

`matrix-desktop-backend` passes broad candidate events into `matrix-desktop-search`. This mirrors the intended `matrix-sdk-search` contract:

- the SDK search layer owns encrypted Tantivy indexes and ngram candidate retrieval;
- `matrix-desktop-search` owns exact verification against the resolved visible event body or attachment filename;
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
- Real Matrix password login.
- SDK sync, room-list, and timeline service wiring.
- Persistent encrypted search indexes.
- E2EE store initialization and recovery UI.
- Video chat.
