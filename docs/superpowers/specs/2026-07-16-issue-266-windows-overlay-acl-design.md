# Windows Overlay ACL Design (#266)

## Goal

Authorize the existing Rust-owned Windows unread overlay path for the `main`
window and prove the built-in Tauri window command crosses the real ACL
boundary. Keep ordinary native backend failures best-effort while reporting ACL
misconfiguration with a distinct, private-data-free diagnostic token.

## Design

`apps/desktop/src-tauri/capabilities/default.json` remains the sole main-window
capability and gains only `core:window:allow-set-overlay-icon`. No wildcard or
unrelated window permission is added.

A Windows-only Tauri test builds the application context with the checked-in
configuration, creates the `main` mock webview, and invokes
`plugin:window|set_overlay_icon` through Tauri IPC for both a non-empty icon and
`None`. This exercises Tauri's generated ACL authority and built-in window
plugin, unlike the existing JavaScript adapter mock. CI runs that focused test
on `windows-latest`.

The TypeScript adapter classifies a rejected overlay call without retaining or
printing the rejection. A Tauri ACL denial emits
`attention_overlay_acl_denied`; all other overlay failures continue to emit
`attention_overlay_failed`. Title, numeric badge, tray, macOS, and Linux paths
are unchanged.

## Verification

- Config contract rejects a missing or non-main overlay permission.
- Windows mock-runtime IPC admits set and clear commands.
- Vitest proves nonzero set, zero clear, ACL classification, and generic
  best-effort failure handling.
- Windows CI evidence is linked from umbrella issue #67 after the PR run.

