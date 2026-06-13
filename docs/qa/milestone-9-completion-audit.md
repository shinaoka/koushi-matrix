# Milestone 9 Completion Audit

Date: 2026-06-12

Scope: `docs/superpowers/plans/2026-06-12-upstream-feedback-roadmap.md` Milestone 9.

## Automated Evidence

| Requirement | Status | Evidence |
| --- | --- | --- |
| Account switching with separate encrypted stores per account/device | Implemented | `SessionKeyId` includes homeserver, user id, and device id; `local_store_paths` derives SDK store, cache, and search index namespaces from that key; account switch dispatch restores the selected saved session. Covered by `local_store_paths_are_namespaced_without_windows_invalid_separators`, `saved_session_infos_from_index_preserves_account_device_identity`, `effects_restore_session_info_returns_first_account_switch_target`, and frontend account switch tests. |
| Homeserver URL validation accepts explicit HTTPS, defaults to HTTPS when omitted, and preserves custom ports | Implemented | `Homeserver::parse` normalizes bare homeservers and preserves explicit ports. Covered by `builds_discovery_url_from_bare_homeserver_name`, `homeserver_input_allows_scheme_omission_and_explicit_port`, and frontend fake API coverage for `matrix.example.org:8448`. |
| Native menu items, window state persistence, context menus, and keyboard shortcuts | Implemented | Tauri menu includes Element-compatible items and platform-standard close/quit; window state persists under `app-shell/window-state.json`; React context menus cover rooms, messages, Spaces, and account actions; shortcut registry drives UI and native accelerators. Covered by Tauri, domain, and component tests. |
| Element-compatible shortcuts mapped into Tauri menu accelerators, including macOS `Cmd+,` and platform close/quit | Implemented | `desktop_menu_items_include_element_compatible_shortcuts` covers `CmdOrCtrl+,`, `CmdOrCtrl+/`, and `CmdOrCtrl+.`. `desktop_menu_items_include_platform_standard_close_and_quit` covers `CmdOrCtrl+W` and `CmdOrCtrl+Q`. |
| Shortcut conflicts resolved across native menus and React handlers | Implemented | `shortcutConflictAudit()` rejects duplicate native accelerators, duplicate global React handlers, missing native accelerators, missing native menu areas, and `adapted` rows without reasons. Covered by `records shortcut conflict resolution and rejects unresolved collisions`. |
| User, room, Space, and Keyboard settings beyond entry-point placeholders | Implemented | `UserSettingsPanel`, `RoomInfoPanel`, `SpaceInfoPanel`, and `KeyboardSettingsPanel` render read-only shell state, summaries, account switch entries, shortcut parity, and Element-like settings entries. User settings expose session/security rows for homeserver, device, per-account local store namespace, OS credential store, and encrypted search index. Room settings expose timeline subscription, exact-verified search index, and DM list scope. Space settings expose child-room membership, global DM list policy, and notification state. Covered by component tests and Browser verification. |
| Element-like context menus where commands exist | Implemented | `contextMenuItems` registry and `ContextMenuSurface` cover message, room, Space, and account/user actions, with destructive styling and viewport clamping. Covered by domain/component tests and Browser verification. |
| Installer/signing preparation for macOS and Windows | Implemented as preparation | Tauri bundle is enabled for `app`, `dmg`, `msi`, and `nsis`; macOS hardened runtime and entitlements are configured; Windows SHA-256 digest, timestamp URL, WiX, NSIS, and stable upgrade code are configured; `release:preflight` validates the setup. |
| Crash-safe recovery for corrupted local stores and search index rebuild | Implemented | Restore retry quarantines SDK store/cache/search index paths after encrypted store restore failure; search index metadata mismatch or invalid path triggers quarantine and rebuild. Covered by Tauri tests. |
| Manual QA scripts for login, restore, recovery, search, edit, redaction, logout, account switch, shortcut parity, right-panel behavior, settings placement, and Space info/settings flows | Implemented | `npm --prefix apps/desktop run qa:manual` emits the Milestone 9 checklist; `releaseScripts.test.ts` verifies every required flow is listed. |

## Environment Gates

These are distribution gates, not code-level Milestone 9 implementation gaps:

- macOS signing/notarization needs Xcode, Apple Developer credentials, and `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID`.
- Windows signing needs a Windows signing certificate or `WINDOWS_SIGN_COMMAND`; Windows MSI/NSIS packaging should be run on Windows CI or a configured cross-signing host.
- A real-account smoke check has verified password login, in-memory persistable session export/import, SDK session restore, one SDK sync, private-data-free room-list counts, private-data-free selected-room timeline item counts after timeline backfill, and logout. Full manual QA must still be executed against a real Matrix account before shipping a release build.
- macOS live OS credential-store ignored tests have been run locally with `cargo test -p matrix-desktop-key --test key_management -- --ignored --nocapture`.
- Windows live OS credential-store ignored tests should be run on Windows before distribution packaging.

## Verification Commands

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo test -p matrix-desktop-key --test key_management -- --ignored --nocapture
cargo run -p matrix-desktop-sdk --features smoke --bin password-login-smoke -- --real-account-qa
npm --prefix apps/desktop test
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run build
npm --prefix apps/desktop run release:preflight
npm --prefix apps/desktop run tauri -- info
git diff --check
```
