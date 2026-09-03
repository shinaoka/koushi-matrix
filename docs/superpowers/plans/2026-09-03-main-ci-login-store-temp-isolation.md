# Main CI — isolate parallel login-store fixtures

## Failure

PR CI run `33706899912` passed the provisional-sync test but failed two parallel `login_store_lifecycle` tests at `login_store_test_support.rs:91` with `store open failed`:

- `empty_crypto_store_from_aborted_attempt_is_distinguished_from_mismatch`
- `identity_mismatch_is_closed_and_original_store_remains_preflightable`

## Root cause

Each integration test calls `login_store_test_support::run` concurrently in one process. `TempRoot::new` constructs a directory from process ID plus `SystemTime::now().as_nanos()`. Wall-clock nanoseconds are not a uniqueness primitive: concurrent calls can observe the same timestamp at the platform clock's actual resolution. The colliding tests then open/delete the same encrypted SQLite store path, explaining simultaneous open failures.

## Deterministic fix

Add a process-local atomic sequence to the directory name, fetched with `Relaxed` ordering because it provides uniqueness only and publishes no data. Keep PID and timestamp for cross-process/stale-run separation. This makes every call unique even when timestamps match.

Do not serialize tests, retry store opens, add sleeps, loosen assertions, or change SDK/store production behavior. This feature-gated test-support module remains the only changed code.

## Verify first and gates

The two simultaneous CI failures are RED. Run the full `login_store_lifecycle` binary 100 times with normal parallel test execution, once with `--test-threads=1` as a serial sanity check, then the full SDK tests, format/diff, independent review, and required CI. Record the second maintenance-task gate separately even though it repairs the already-open CI stabilization PR.

## Review record

- Design review: `reviewer-flash` **Correct-to-merge**. The repeat gate is quantified at 100 parallel-harness runs plus one serial sanity run. A similar PID+millisecond smoke-binary path is intentionally unchanged: it creates only one root per process, so PID is sufficient and no intra-process collision exists.
- Verify-first evidence: PR CI `33706899912` failed two concurrent store opens before the fix. The full parallel-harness binary passed 100/100 runs, its serial sanity run passed 7/7, and full SDK tests passed all unit/integration/doc-test binaries.
- Implementation review: `reviewer-flash` **Correct-to-merge**, no findings.
