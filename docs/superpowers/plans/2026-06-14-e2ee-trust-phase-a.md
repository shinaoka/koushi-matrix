# 2026-06-14 E2EE Trust Phase A

Goal: start Issue #13 with Rust-owned trust, verification, and key-backup
state-machine contracts before any GUI work.

Canon consulted:

- `REPOSITORY_RULES.md`
- `docs/architecture/overview.md`
- `docs/architecture/state-machine.md`
- `docs/policies/engineering-rules.md`
- `docs/superpowers/plans/2026-06-13-roadmap-phases-10-18.md`

## Plan

1. Add RED reducer tests for an account-level E2EE trust state slot:
   verification flow request/accept/SAS/confirm/done/cancel/failure, guarded by
   ready session and request correlation.
2. Add cross-signing and key-backup status DTOs to Rust state, with private
   data-free failure kinds and no key material.
3. Add account-level command/event/effect contract types for verification,
   cross-signing bootstrap, key backup enable/restore, and identity reset.
4. Update state-machine diagrams and architecture docs in the same change.
5. Run focused Rust state/core checks and leave a GitHub issue work record.

## Phase Boundary

This slice does not call Matrix SDK verification or key-backup APIs yet and
does not add React controls. It establishes the Rust-owned state and typed
command/event surface that later AccountActor and GUI work must consume.

## Verification

Run at minimum:

```bash
cargo test -p matrix-desktop-state --test e2ee_trust_state
cargo test -p matrix-desktop-core
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml dto::tests
```

Before merge, also run formatting, `qa:wasm-check`, secret scan, and
`git diff --check`.
