# SAS Delivery Observability Design

## Problem

An unverified Koushi session can start “Verify with another device,” remain in the verifying state for 120 seconds, and then report `Timeout` while an existing Element X session receives no verification request. The current implementation records neither the outgoing request boundary nor the Matrix SDK request states. It therefore cannot distinguish an empty or unexpected recipient set, an accepted to-device send followed by no remote response, a restricted-sync delivery failure, an SDK cancellation, or the app-owned timeout.

The recovery-code path succeeds in the same clean-data environment, so this work remains inside issue #244's shared verification-admission lifecycle. It must not add retries, extend timeouts, or create a second promotion path.

## Design

Add a privacy-safe SAS diagnostic boundary spanning the SDK adapter and `AccountActor`. Every event is recorded in the bounded `koushi-diagnostics` buffer and written to stderr through the existing dual-sink helper.

The boundary records these stages:

- `request_started`
- `recipients_resolved`, with total other-device count and eligible recipient count
- `request_send_finished`, with a closed success/failure outcome and initial request state
- `request_state_changed`, with a closed request-state token
- `sas_start_attempted` and `sas_start_finished`
- `sas_state_changed`, with a closed SAS-state token
- `restricted_sync_succeeded` and `restricted_sync_failed` while an own-user flow is active
- `observer_ended`
- `timeout_fired`
- `settled`, with a closed terminal kind

All events carry the app-owned numeric flow ID. Events emitted before the flow is stored also carry that flow ID supplied by `AccountActor`; SDK transaction IDs are not exposed. Request cancellation retains the SDK's closed cancellation category (`timeout`, `user`, `accepted_elsewhere`, `unknown_method`, `key_mismatch`, or `other`) instead of discarding it before diagnostics, while the existing reducer failure mapping remains unchanged unless evidence later justifies a behavioral correction.

The SDK adapter returns a small request-start report with privacy-safe counts and the opaque request handle. It does not expose raw device IDs. The eligible count must describe the same recipient predicate used by the Matrix SDK so the diagnostic cannot claim that a blocked or otherwise excluded device was targeted.

## Privacy and Failure Handling

Diagnostics may contain only numeric flow correlation, counts, booleans, elapsed time, and closed enum tokens. User IDs, device IDs, device display names, homeserver URLs, Matrix transaction IDs, access tokens, recovery material, SDK error strings, and message content are forbidden.

Diagnostic emission is observational: it must not alter request ordering, await behavior, cancellation, the 120-second timeout, or promotion. Sink failure must not affect verification.

## Testing

Tests must first fail for the missing observations, then prove:

1. Request creation reports recipient counts and the initial closed request state without raw identifiers.
2. Own-user request and SAS state changes remain correlated to the same flow ID.
3. Restricted-sync success/failure is logged only with the active own-user flow correlation.
4. Observer termination and the app timeout are distinguishable.
5. SDK cancellation categories survive long enough to be diagnosed while reducer behavior remains compatible.
6. Diagnostics and stderr receive identical structured records through the shared sink.
7. Existing verification admission, recovery, SAS, frontend, diagnostics, and release-gate tests remain green.

The next real-device run should be sufficient to classify the failure as recipient selection, send failure, missing remote response, missing restricted-sync delivery, remote cancellation, or local timeout.
