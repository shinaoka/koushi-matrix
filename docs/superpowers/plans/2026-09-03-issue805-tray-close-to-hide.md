# Issue #805 — Tray Icon And Cross-Platform Close-To-Hide

Starts from `origin/main` `809360cdf580c45f18394d8f0339b7834120e4d0` with Matrix
SDK gitlink `600044a4c5f621863e1fca3b33eed59aef85a13f`.

## Objective

Linux and Windows builds have no tray icon, and closing the product window
destroys it and ends the process. Only macOS hides on close. Give every desktop
platform a tray icon with Show and Quit, make close-to-hide the default
everywhere, and make explicit Quit the single path that shuts the core runtime
down as part of process exit.

## Governing contract

`docs/architecture/overview.md`:

- `Desktop Window Lifecycle And Tray` (added by this change) — the tray is an
  adapter-owned platform capability resolved truthfully at startup;
  close-to-hide is the default on all three platforms, gated on Linux and
  Windows by `SettingsValues.window.close_to_tray` (default `true`) plus actual
  tray availability; explicit Quit is the only path that triggers
  `AppCommand::Shutdown` as part of exit.
- `Desktop Attention Surfaces` — `native_attention_capabilities_for_platform` is
  the platform-static baseline; runtime-decided capabilities stay `Unknown`
  there and the adapter overwrites them in the DTO projection. `tray` is the
  first such capability.

## Root cause

`apps/desktop/src-tauri/src/lib.rs`:

- the `CloseRequested` hide branch in `on_window_event` is behind
  `#[cfg(target_os = "macos")]`, so on Linux and Windows the close falls through
  to geometry persistence and the window is destroyed;
- `window_event_should_stop_background_tasks` matches only
  `WindowEvent::Destroyed`, so core shutdown is coupled to window destruction
  rather than to an exit request. With close-to-hide enabled the window is never
  destroyed, so shutdown would never be submitted; and on macOS today menu Quit
  can exit without ever destroying the window;
- no `TrayIconBuilder` is registered anywhere, and the `tauri` dependency does
  not enable the `tray-icon` feature;
- `native_attention_capabilities_for_platform` hardcodes
  `tray: NativeAttentionCapability::Unknown` with no mechanism for the adapter
  to report the real answer.

## Minimal design

1. **Tray ownership** — new `apps/desktop/src-tauri/src/tray.rs`. One
   process-wide tray built in `setup` with a two-item menu (Show Koushi, Quit
   Koushi), tooltip `Koushi`, and the app's default window icon. Left-click
   (where the platform delivers it) and Show both call the existing
   `ensure_main_window_visible_for_handle`. Build failure is recorded as a
   `desktop.lifecycle` diagnostic and downgraded to "no tray"; the app keeps
   running windowed.

2. **Truthful capability resolution** — the tray outcome is a process-level fact,
   exactly like `frontend_display_platform()`, so it is stored in a module-level
   atomic in `tray.rs` (`Unknown` before `setup` completes, then `Available` or
   `Unavailable`). `koushi-state` gains
   `NativeAttentionCapabilities::with_tray`, and both DTO projection sites in
   `dto.rs` apply `.with_tray(tray::observed_tray_capability())` to the
   platform-static baseline. No new transport, no new core command: the value
   never needs to reach the reducer, only the snapshot React reads.

3. **Close decision** — pure
   `close_requested_action(tray_available: bool, close_to_tray: bool) ->
   CloseRequestedAction`. On non-macOS `CloseRequested`, the handler reads
   `close_to_tray` from a snapshot and hides (after
   `persist_close_window_state_if_ready` and `api.prevent_close()`) when the
   action is `HideToTray`, mirroring the macOS diagnostic. Reading the setting
   must be synchronous because `prevent_close` cannot be deferred across an
   await, so `CoreRuntimeState` holds a dedicated `CoreConnection` whose
   `snapshot()` is a sync latest-wins read.

4. **Quit path** — pure `quit_request_action(stage) -> QuitRequestAction` over a
   three-state `QuitStage` (`Idle`, `ShuttingDown`, `ShutdownComplete`) kept in
   `CoreRuntimeState`. `RunEvent::ExitRequested` calls `api.prevent_exit()` and
   submits `AppCommand::Shutdown` on the first request, prevents exit again
   while shutdown is in flight, and lets exit proceed once shutdown has
   completed. Tray Quit and the predefined menu Quit both funnel into
   `app.exit(0)`, so both are graceful and shutdown is submitted exactly once.
   The `Destroyed` path keeps its existing `submit_core_shutdown` call for the
   case where the window really is destroyed.

5. **Setting** — new `WindowSettings { close_to_tray: bool }` on
   `SettingsValues` (`#[serde(default)]`, default `true`) plus a `window` arm on
   `SettingsPatch` and `apply_patch`. Persistence is whole-`SettingsValues`
   serde in `koushi-core/src/settings.rs`, so it round-trips for free and old
   settings files backfill the default. Frontend gets a `WindowSettings`
   interface and one toggle row in a new Window section of
   `UserSettingsPanel.tsx`.

6. **Single instance** — the existing `tauri_plugin_single_instance` callback
   already calls `ensure_main_window_visible_for_handle`, so a second launch
   un-hides the window with no change needed.

## Files

- `docs/architecture/overview.md` — canon amendments above.
- `apps/desktop/src-tauri/Cargo.toml` — `tauri` gains the `tray-icon` feature.
- `apps/desktop/src-tauri/src/tray.rs` — new.
- `apps/desktop/src-tauri/src/lib.rs` — tray registration, close decision,
  exit-request handling, `CoreRuntimeState` fields.
- `apps/desktop/src-tauri/src/dto.rs` — `.with_tray(...)` at both projections.
- `apps/desktop/src-tauri/src/tests.rs` — decision-function unit tests.
- `crates/koushi-state/src/state/native_attention.rs` — `with_tray`.
- `crates/koushi-state/src/state/settings.rs` — `WindowSettings`, patch arm.
- `crates/koushi-state/tests/attention_surface.rs` — baseline/override tests.
- `apps/desktop/src/domain/types.ts`,
  `apps/desktop/src/components/UserSettingsPanel.tsx`,
  `apps/desktop/src/i18n/messages.ts` — toggle row.

## Verification

- `cargo test -p koushi-state --lib`
- `cargo test -p koushi-state --test attention_surface`
- `cargo test -p koushi-desktop`
- `cargo check -p koushi-desktop --all-targets` (proves `tray-icon` compiles)
- `npm run typecheck` / `npm run lint` in `apps/desktop`

## Accepted limitations

- The close-to-hide gate reads `close_to_tray` from a latest-wins snapshot, so a
  settings change that has not yet been published when the user clicks the close
  button uses the previous value. The next close uses the new value; this is the
  same latest-wins contract every other adapter snapshot read has.
- Tray availability is observed once, at startup. A status-notifier host that
  appears or disappears later does not re-resolve the capability. Re-resolution
  would need a tray-lifecycle watcher and is deliberately out of scope.
- The tray menu labels are English literals, like the existing native menu in
  `desktop_menu.rs`. Native menu localization is a separate, pre-existing gap.
