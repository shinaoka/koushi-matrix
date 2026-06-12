# Search Ngram Spike

Status: in progress

Goal: prove configurable ngram search in `matrix-sdk-search` with Japanese/CJK mixed text, encrypted index opening, rebuild behavior, edits, redactions, and late decryption.

SDK branch: `search-ngram`
SDK path: `vendor/matrix-rust-sdk`

Acceptance:
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml ngram`
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml search_index --features experimental-search,sqlite,e2e-encryption`

## Result

- Ngram tokenizer config is implemented in `matrix-sdk-search`.
- Redaction fallback removes by redacted event ID even if the original event is not present in the current cache.
- Event-cache lag is detected and logged as requiring room reindex.
- Full late-decryption reindex remains a separate implementation task after the spike because current SDK event-cache task only skips UTD events and does not emit a late-decryption reindex operation.
