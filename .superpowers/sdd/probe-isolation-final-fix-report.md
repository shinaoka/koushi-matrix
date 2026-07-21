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
