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
        +--> crates/matrix-desktop-state
        +--> crates/matrix-desktop-search
        +--> crates/matrix-desktop-key
```

`matrix-desktop-backend` intentionally has no network client. It executes the same loop the real backend will use:

```text
AppAction -> reduce(AppState) -> AppEffect -> backend effect runner -> follow-up AppAction
```

The fake effect runner handles session restore, sync start, timeline subscription, thread subscription, sending synthetic local text, and search. Real Matrix integration should replace this runner with a Matrix SDK runner without moving state transitions into the UI.

## Login Boundary

The next real-login step should attach at these points:

1. `AppEffect::Login` creates a `matrix_sdk::Client` using the configured homeserver.
2. Successful login dispatches `AppAction::LoginSucceeded(SessionInfo)`.
3. The backend creates a `SessionKeyId` from homeserver, user id, and device id.
4. `matrix-desktop-key` loads or creates the local unlock secret through the OS credential store.
5. The SDK store key and search index key are derived from that local unlock secret.
6. `AppEffect::StartSync` starts SDK sync, room-list services, timeline subscriptions, and search indexing.

The default homeserver remains `https://matrix.org`, but `FakeDesktopBackendConfig` already keeps homeserver configurable. The real login UI should expose the same setting before submitting credentials.

## Pre-Login Shell

The app now has an explicit first-run path before real Matrix login:

1. `AppEffect::RestoreSession` may resolve to `AppAction::RestoreSessionNotFound`.
2. The reducer enters `SessionState::SignedOut` without recording an error.
3. The React shell renders a homeserver-configurable sign-in form instead of the Slack-like ready surface.
4. `submit_login` dispatches `AppAction::LoginSubmitted`, which emits `AppEffect::Login`.
5. The fake backend intentionally turns that effect into `AppAction::LoginFailed` with a non-secret message because real Matrix login is not wired yet.

Open the browser shell in first-run mode with:

```bash
http://127.0.0.1:5173/?session=signed-out
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
- Real Matrix login.
- SDK sync, room-list, and timeline service wiring.
- Persistent encrypted search indexes.
- E2EE store initialization and recovery UI.
- Video chat.
