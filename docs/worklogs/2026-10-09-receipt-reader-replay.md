# Receipt Reader Replay Identity

## Scope

Restore the existing timeline replay contract so a returning subscriber keeps
the actor-owned projection identity needed to open the full read-receipt reader
scope. The compact receipt summary remains unchanged.

## Canon consulted

- `REPOSITORY_RULES.md`: root-cause fixes, Rust ownership, reducer/state-machine
  discipline, verification, and review.
- `docs/architecture/state-machine.md`: an active timeline replay carries the
  retained Core projection identity.
- `docs/agents/state-ownership.md`: Rust owns live receipt data and timeline
  navigation projection.
- `docs/agents/verification.md`: reproduce behavior headlessly before the fix.

No canon amendment or user-guide change is needed: this restores the documented
replay behavior without adding UI, text, settings, or a new state transition.

## Change

`InitialItemsRequestIdentity::replay` now always carries the stable projection
request ID owned by the active timeline actor, even after its initial projection
has committed. The command that triggered a replay remains separately correlated
by `cause_request_id`.

## Verification

- RED: `cargo test -p koushi-core --lib committed_replay_retains_actor_projection_identity`
  failed because the committed replay emitted no projection identity.
- GREEN: the same focused test passes after the Core fix.
- `cargo test -p koushi-core --lib`
- `cargo test --profile ci --workspace --exclude sidebar-composition --exclude key-management`
- `cargo fmt --check`
- All three required clippy lanes, including the release workspace lane
- Desktop typecheck, lint, full test suite, dependency audit, and secret scan
- `qa:headless-local -- --server=both` against Tuwunel and Synapse
- SDK submodule, Rust test-structure, user-help, and diff checks
