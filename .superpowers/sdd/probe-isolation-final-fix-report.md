# Probe isolation final-review fixes

## Files changed

- `docs/policies/engineering-rules.md`
- `docs/architecture/overview.md`
- `docs/architecture/state-machine.md`
- `AGENTS.md`
- `crates/koushi-core/src/sync.rs`

The four contract passages now consistently state that the disposable probe has
no refresh token, automatic refresh is impossible/disabled, retries are
disabled, and its single two-second deadline covers client setup plus one
transport request. They retain cursor/payload discard, anti-fingerprinting,
and no-second-owner requirements, and require behavioral coverage for success,
omission, malformed/error, timeout, and `M_UNKNOWN_TOKEN` isolation
(zero refresh calls, no authoritative session/token mutation, and fail-closed
`LegacySync` selection).

The targeted continuation test now drains the broadcast receiver through
`TryRecvError::Empty`, fails on every `Stopped` or `Failed` event, and treats
`Lagged` and `Closed` as explicit failures.

## Verification

All commands were run from the probe-isolation worktree.

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed (exit 0; repository rustfmt configuration emitted existing stable-toolchain warnings for nightly-only options). |
| `cargo test -p koushi-core --lib unknown_isolated_invite_probe_falls_back_and_commits_first_legacy_response -- --nocapture` | Passed: 1 passed, 0 failed. |
| `cargo test -p koushi-sdk --lib sliding_sync_invite_probe -- --nocapture` | Passed: 6 passed, 0 failed. |
| `git diff --check` | Passed (exit 0). |
| `node scripts/desktop-secret-scan.mjs` | Passed: `secret scan ok (tracked files)`. |

## Concerns

None. The change is confined to the four requested documentation passages, the
requested test body, and this required report; no production Core code or SDK
code changed.

## Follow-up: bound isolated probe setup

`probe_sliding_sync_invite_list_support` now applies one outer
`tokio::time::timeout(SYNC_INVITE_PROBE_TIMEOUT, async { ... })` around both
disposable-client build/restore and `send_sliding_sync_invite_list_probe`.
An outer timeout returns `Unknown`; setup failure continues to return `Unknown`.
The existing request configuration, including its request timeout and
`disable_retry()`, is unchanged.

### RED/GREEN evidence

1. **RED** — added the structural guard to require the public probe's outer
   timeout to precede both the build and send calls.
   `cargo test -p koushi-sdk --lib sliding_sync_invite_probe_contract_is_typed_bounded_and_discards_cursor -- --nocapture`
   failed as expected with: `the public probe must start one outer end-to-end timeout`.
2. **GREEN** — wrapped the existing build/restore and send sequence in the one
   outer timeout, mapping `Err(_)` to `MatrixSlidingSyncInviteListSupport::Unknown`.
   The same focused guard then passed: 1 passed, 0 failed.

### Follow-up verification

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed (exit 0; existing stable-toolchain warnings for nightly-only rustfmt options). |
| `cargo test -p koushi-sdk --lib sliding_sync_invite_probe -- --nocapture` | Passed: 6 passed, 0 failed. |
| `cargo test -p koushi-core --lib unknown_isolated_invite_probe_falls_back_and_commits_first_legacy_response -- --nocapture` | Passed: 1 passed, 0 failed. |
| `git diff --check` | Passed (exit 0). |
| `node scripts/desktop-secret-scan.mjs` | Passed: `secret scan ok (tracked files)`. |

### Follow-up concerns

None. The code change is confined to `crates/koushi-sdk/src/lib.rs`; this report
was appended as requested.
