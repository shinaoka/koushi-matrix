# Prerequisite Spike Results

Date: 2026-06-11

## Search

Result: pass

Evidence:
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml ngram -- --nocapture`
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml -- --nocapture`
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml search_index --features experimental-search -- --nocapture`

Remaining implementation work:
- SDK event-cache integration with `matrix-desktop-search`;
- full late-decryption reindex path connected to the adapter;
- full index rebuild UI and progress reporting.

Follow-up completed:
- `matrix-desktop-search` provides in-memory exact verification, UTF-16
  highlights, attachment filename matches, pending edit handling, and redaction
  cleanup before results are sent to the UI.

## Sidebar Composition

Result: pass

Evidence:
- `cargo test -p sidebar-composition`

Remaining implementation work:
- replace spike inputs with SDK stream adapters;
- add nested Space and multi-parent room UI decisions.

## Key Management

Result: pass

Evidence:
- `cargo test -p key-management`

Remaining implementation work:
- run ignored credential-store test on Windows;
- integrate `SqliteStoreConfig::key` in the Tauri backend;
- define logout and missing-secret UI.
