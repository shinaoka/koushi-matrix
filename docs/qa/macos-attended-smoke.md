# macOS Attended Smoke Checklist

macOS is now reserved for attended, macOS-specific validation. The primary
unattended GUI lane is `qa:linux-gui` on Linux.

Use this checklist only when you need to confirm platform-specific native
behavior on macOS:

- [ ] WKWebView rendering spot-check
- [ ] OS menu accelerators respond as expected
- [ ] Keychain prompt suppression remains in effect for QA runs
- [ ] Signing, notarization, and Gatekeeper launch behavior when applicable

Credential-store verification is split by tier:

- Tier 1 is generic and headless on any OS:
  `cargo test -p matrix-desktop-key credential_backend`,
  `cargo test -p matrix-desktop-state --test local_encryption_state`, and the
  core StoreActor health tests. This tier uses the in-memory/fake credential
  backend and does not touch the OS keychain.
- Tier 2 is macOS-only, unattended on a real macOS CI/session, and opt-in:
  `MATRIX_DESKTOP_MACOS_KEYCHAIN_QA=1 cargo test -p matrix-desktop-key credential_backend_macos_temporary_keychain_round_trip_is_env_gated -- --nocapture`.
  The test creates a temporary keychain with `security create-keychain`,
  performs one synthetic set/get/delete through the normal `keyring` backend,
  and verifies locked-keychain failure maps to a coarse fail-closed state. It is
  not an Xvfb-style headless lane; macOS has no equivalent.
- Tier 3 remains attended-only: native consent dialogs, Touch ID, locked
  login-keychain UX, and signed-build ACL behavior. `tauri-driver` does not
  support native macOS GUI automation, so do not claim automated coverage for
  this tier.

Privacy and handling rules:

- Do not store post-login screenshots unless they were explicitly approved
  for the test case.
- For real accounts, rely on private-data-free QA window-title tokens instead
  of screenshots.
- Keep any captured evidence free of message bodies, room names, Matrix IDs,
  and attachment names unless the test was approved to collect them.
