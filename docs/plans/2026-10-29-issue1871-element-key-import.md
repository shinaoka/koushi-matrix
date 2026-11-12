# Issue #1871 — local Element / Element X key import (Phase A)

Date: 2026-10-29. Issue: shinaoka/koushi-matrix#1871. Base: `bf2279d1`.

## Canon consulted

- `REPOSITORY_RULES.md` — Architecture And Ownership (adapter is a transport /
  platform layer and never calls SDK APIs; product semantics live in Core),
  Security Rules (key material never logged or crossing to the webview), Tests
  And Fixtures (synthetic data only).
- `docs/architecture/overview.md`, `docs/agents/verification.md` — command /
  event ownership and the RED-then-GREEN requirement.
- `docs/architecture/element-key-import.md` — the new canon entry for this
  feature.

## Discovery that changed the plan

The issue records Element X's crypto store as plaintext pickles. Re-checking the
byte pattern it quotes (`83 A7 76 65…`) against the vendored SDK's store
encoding shows `fixmap(3)` + `"version"`: the SDK's encrypted store value
(`EncryptedValue { version, ciphertext, nonce }`), and `matrix-sdk-sqlite`
encrypts every value when a store passphrase is set. Element X generates that
passphrase at sign-in and keeps it in its own keychain. So a shipped Element X
store is *not* readable from outside the app, and a direct SQLite import was
dropped after it existed in the branch: the store detection reports how much data
is there and that it cannot be imported, and those accounts migrate by verifying
from the old device with secret sharing instead.

The Element Desktop (CDP) path is the local source that works today.

## Boundaries

- `apps/desktop/src-tauri/src/migration/` — platform capability: detection, the
  CDP export, and the `0600` staging file.
- `koushi-sdk::import_migrated_room_keys` — the Element-format envelope, reusing
  `import_room_keys`.
- `koushi-protocol` / `koushi-core` — `AccountCommand::ImportMigratedRoomKeys`,
  which reuses the existing `RoomKeyImportRequested`/`RoomKeyImported`/
  `RoomKeyImportFailed` state machine, so no reducer change is needed.
- `apps/desktop/src-tauri/src/commands/migration.rs` — the two IPC commands,
  registered in `lib.rs`.

## Verification

See the PR body for the exact gate output. Coverage added:

- `apps/desktop/src-tauri/src/migration/element_x_tests.rs` — detection over
  synthetic session directories (store present, signed-out directory, missing
  root, protected container).
- `apps/desktop/src-tauri/src/migration/element_desktop_tests.rs` — the CDP
  conversation against a fake loopback server (success, export exception, no
  webapp target, non-loopback target refused).
- `crates/koushi-sdk/src/e2ee/migrated_room_keys_tests.rs` — a staged Element
  export imports through the manual path; an empty staging file is a no-op; an
  unknown staging version is rejected.

## Left open

- Phase B (React surface, message-catalog copy, browser tests) is not part of
  this change.
- No automated test reads a real Element or Element X store; manual confirmation
  against a signed-in Element installation has not been done.
- Reading an Element X passphrase from the app's keychain is out of scope, and
  Element X's store is now explicitly reported as unimportable rather than
  parsed. Those accounts migrate by verifying from the old device and sharing
  secrets in Koushi; starting that automatically is Phase B.
