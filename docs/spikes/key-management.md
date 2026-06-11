# Key Management Spike

Status: in progress

Goal: prove local unlock secret lifecycle for macOS Keychain and Windows Credential Manager compatible storage.

Acceptance:
- `cargo test -p key-management`
- ignored live credential-store tests pass manually on macOS and Windows.

## Result

- Local unlock secret is 32 random bytes.
- SDK store key is a 32-byte raw key suitable for `SqliteStoreConfig::key`.
- Search key is a namespaced string suitable for `SearchIndexStoreKind::EncryptedDirectory`.
- Credential-store records are named by homeserver, user ID, and device ID.
- Missing credential handling must fail closed and offer local-state reset.
