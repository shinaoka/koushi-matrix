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

The first slice did not call Matrix SDK verification or key-backup APIs and did
not add React controls. It established the Rust-owned state and typed
command/event surface that later AccountActor and GUI work must consume.

The follow-up SDK bridge slice keeps the same Phase A boundary: production
`CoreCommand::Account` trust commands project reducer pending state before
`AccountActor` routing, and `AccountActor` calls `koushi-sdk`
private-data-free wrappers for cross-signing bootstrap and key-backup enable.
Identity reset now calls the SDK wrapper and projects immediate completion or a
typed `AwaitingAuth` state with only UIAA/OAuth/unknown auth kind; the SDK
continuation handle stays inside `AccountActor`. The continuation slice adds
`SubmitIdentityResetAuth` so OAuth approval and UIAA password submission
also project through the reducer before the actor calls the SDK handle. Device
verification, local homeserver QA tokens, and all GUI controls remain outside
this slice.

The key-backup restore slice adds the secret-bearing `RestoreKeyBackup`
`CoreCommand::Account` payload while keeping the projected reducer action,
events, effects, and snapshots secret-free. The SDK wrapper uses public
matrix-rust-sdk APIs only: recover/import secrets, then hydrate currently joined
rooms via `Backups::download_room_keys_for_room`. Progress counters describe the
joined-room hydration set, not a backup-wide exhaustive import. True all-session
backup-wide restore remains a later SDK API/patch decision and must be proven
with local homeserver QA before #13 closure.

The device-verification bridge slice wires outgoing device verification to
public matrix-rust-sdk APIs without GUI changes. `koushi-sdk` exposes
opaque verification-request and SAS handles plus private-data-free state/emoji
DTO mapping; `AccountActor` owns those handles, observes SDK request/SAS state
streams, and projects reducer actions / `CoreEvent::E2eeTrust` updates. The
mismatch-cancel slice adds `VerificationCancelReason` so plain user cancel
returns verification to `Idle`, while SAS mismatch calls the SDK mismatch path
and settles the reducer as kind-only `Mismatch` failure. The incoming request
discovery slice adds a Rust-owned SDK observer in `AccountActor` and separates
verification `flow_id` from command `request_id`, so SDK-originated requests can
be accepted/cancelled later without React owning discovery or handle state.

The local proof slice adds the core QA `e2ee_trust` scenario. It runs through
`CoreCommand`/`CoreEvent` only, logs the same synthetic user into a second local
data directory/device, and proves cross-signing bootstrap, encrypted seed-room
backup upload, passphrase-backed key-backup enable, wrong-secret restore
failure, successful joined-room restore on the second device, same-user SAS
device verification, and identity reset before any GUI wiring. The slice also
separates identity-reset auth continuation command `request_id` from the
Rust-owned identity-reset `flow_id`,
matching the verification follow-up model so React never owns pending trust
state. The proof uses the probed SyncService core backend; `AccountActor`
auto-accepts peer SAS only when the SDK SAS state is `Started`, while `Created`
remains the local post-`start_sas` side. The same-user local proof keeps the
request direction A2 -> A and starts SAS from requester A2; starting from the
accepting device reproduced Tuwunel `m.key_mismatch` cancellation before SAS
emoji presentation. It starts the request while continuous sync is running,
then pauses both sync loops and drives SAS delivery with bounded `SyncOnce`
polling; overlapping continuous SyncService delivery with manual `SyncOnce`
nudges reproduced pre-SAS key-mismatch flakes. The headless runner registers
separate synthetic users for the SDK lane and each core backend leg so the E2EE
proof's device graph is isolated. GUI surfaces remain Phase B work, and
`RestoreKeyBackup` is intentionally not runtime-gated to `SessionState::Ready`
only because a fresh second device can move to `NeedsRecovery` before restore.
backup-wide all-session restore semantics still require a public SDK API or
reviewed vendored patch before the product can claim exhaustive restore.

## Verification

Run at minimum:

```bash
cargo test -p koushi-state --test e2ee_trust_state
cargo test -p koushi-sdk e2ee_trust_tests
cargo test -p koushi-core
cargo test -p koushi-core --features qa-bin --bin headless-core-qa
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --core --core-backend=probed --scenario=e2ee_trust --timeout-ms=240000
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml dto::tests
```

Before merge, also run formatting, `qa:wasm-check`, secret scan, and
`git diff --check`.
