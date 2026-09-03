# Main CI — phase-fence provisional encryption-sync test

## Failure

Main CI run `33704099834` failed only `runtime_e2ee::provisional_verification_hands_one_encryption_sync_owner_to_normal_runtime`: the single 5-second deadline around the first Simplified Sliding Sync request elapsed. The same test passed in the required PR run for the identical product tree.

## Root cause

The test submits password login and immediately starts one deadline that conflates two independently asynchronous phases:

1. versions/login/client+crypto-store initialization and projection into provisional verification state;
2. provisional verification starting its encryption-only Simplified Sliding Sync owner and issuing the first request.

The test intends to verify phase 2 ownership, but its guard also budgets all incidental phase 1 setup. Under CI scheduling/load, setup can consume the deadline before the owned request is observable. The oneshot responder itself is already an event-driven phase-2 signal.

## Deterministic fix

After submitting login, first use the existing non-replaying-safe `CoreConnection` snapshot/event fence to await the authoritative provisional/awaiting-verification session state. Only then apply the existing 5-second deadlock guard to `first_request_rx`.

The first PR CI run (`33705267718`) then proved the phase-1 state transition itself can be starved for the full guard under the default single-thread Tokio test runtime while the Matrix client, Wiremock server, crypto initialization, and Core actors share that executor. Run this one integration test with Tokio's two-worker multi-thread flavor so blocking/cooperative startup work cannot starve the in-process HTTP server and actor progress. This changes test execution topology, not any deadline or product behavior.

This is not a retry and does not increase either phase's guard: both remain 5 seconds. The two-worker executor removes the single-thread starvation mechanism rather than extending tolerance.  it gives each causally distinct phase its established bounded event-driven assertion and makes failures identify the missing transition. The oneshot is installed before login, so a fast request remains buffered and cannot be lost while awaiting state.

Do not change production startup, timeout, retry, sync ownership, Matrix SDK behavior, or test-server responses.

## Verify first and gates

The failing main run is RED. Run the exact test repeatedly, then full `runtime_e2ee`, Core tests, format/diff, and GitHub CI. Merge as a separate maintenance PR and require green main CI before Issue #785 is complete. If the same phase fence recurs, do not add retries or raise guards; isolate this networked integration test into a dedicated binary/process before reassessing the runtime path.

## Review record

- Design review: `reviewer-flash` **Correct-to-merge**. Implementation is pinned to `Provisional | AwaitingVerification`, keeps distinct phase diagnostics, and comments that the preinstalled oneshot buffers an early request.
- Verify-first evidence: main CI run `33704099834` failed before the change. The exact phased test passed, then its compiled binary passed 20/20 repetitions; full `runtime_e2ee` passed 2/2. After the CI-exposed executor adjustment, the exact multi-thread test passed 50/50 repetitions and the full binary passed 2/2.
- Implementation review Round 1: `reviewer-flash` **Correct-to-merge**, no findings, but PR CI `33705267718` then failed the newly isolated phase-1 state fence at exactly 5 seconds. The second design step uses a two-worker Tokio test runtime without changing either guard. Design re-review: `reviewer-flash` **Correct-to-merge**; it confirmed production/QA topology alignment and no Send/global-state hazard. Final implementation re-review: **Correct-to-merge**; the only procedural finding was to commit this annotation and updated record before rerunning PR CI.
