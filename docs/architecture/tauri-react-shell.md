# Tauri React Shell

Status: historical foundation. The current production runtime architecture is
documented in [overview.md](./overview.md).

Date: 2026-06-11

`apps/desktop` was the first packaged desktop app surface. This page describes
the early Tauri v2 + React shell that rendered the same pre-login fake session
before the headless core runtime landed. The current transport contract is
`CoreCommand` / `CoreEvent` in [overview.md](./overview.md).

## Boundaries

```text
React components
  render snapshot DTOs and send user intent
        |
        v
DesktopApi
  Tauri runtime: @tauri-apps/api/core.invoke
  Browser runtime: in-memory fake API
        |
        v
src-tauri commands
  Mutex<FakeDesktopBackend>  historical shell backend
        |
        v
matrix-desktop-backend
```

The React layer does not import Matrix SDK types and does not own Matrix state
transitions. It renders `DesktopSnapshot` DTOs and calls commands such as
`select_room`, `open_thread`, `close_thread`, and `submit_search`.

## Browser Fallback

When the app is opened through Vite outside Tauri,
`createDesktopApi()` uses `createBrowserFakeApi()`. This lets UI work continue
without a native app process while keeping the same DTO shape as the Tauri
commands.

The fallback supports:

- Space switching;
- global DMs;
- room selection;
- thread open/close;
- exact search;
- attachment filename search.

## Tauri Commands

The Tauri command layer normalizes Rust enum state into frontend DTOs:

- `SessionState::Ready` becomes `{ "kind": "ready", "homeserver": "...", ... }`.
- `SearchState::Results` becomes `{ "kind": "results", "results": [...] }`.
- search match fields are serialized as `messageBody` or `attachmentFileName`.

This avoids leaking Rust's default externally tagged enum serialization into TypeScript. The DTO contract is covered by a Rust unit test in `apps/desktop/src-tauri/src/dto.rs`.

## Run

Browser fallback:

```bash
cd apps/desktop
npm install
npm run dev
```

Open `http://127.0.0.1:5173/`.

Tauri development shell:

```bash
cd apps/desktop
npm run tauri dev
```

Current Tauri build settings follow Tauri v2's `devUrl` and `frontendDist` configuration model.

## Next Step

Historical note: the next step in this shell-era document was to replace fake
commands with a real Matrix SDK runner. That work has since landed in the
current core runtime and should be read through `overview.md` instead.
