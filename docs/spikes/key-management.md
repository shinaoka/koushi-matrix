# Key Management Spike

Status: in progress

Goal: prove local unlock secret lifecycle for macOS Keychain and Windows Credential Manager compatible storage.

Acceptance:
- `cargo test -p key-management`
- ignored live credential-store tests pass manually on macOS and Windows.
