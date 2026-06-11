# Search Ngram Spike

Status: in progress

Goal: prove configurable ngram search in `matrix-sdk-search` with Japanese/CJK mixed text, encrypted index opening, rebuild behavior, edits, redactions, and late decryption.

SDK branch: `shinaoka/search-ngram`
SDK path: `vendor/matrix-rust-sdk`

Acceptance:
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml ngram`
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml search_index --features experimental-search,sqlite,e2e-encryption`
