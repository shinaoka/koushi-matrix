# Verification Preparation Observability Design

## Problem

After recovery-code verification, the desktop can remain on “Preparing your rooms…” while `promotion_full_state_sync_once` awaits a full-state `/sync`. The current path reduces the SDK result to a boolean and emits no diagnostic event before, during, or after that await. A field report therefore cannot distinguish a live request, an SDK failure, a cancelled task, or a stale completion.

## Design

Add a verification-admission diagnostic boundary in `koushi-core`. Every event at this boundary is recorded in the existing bounded `koushi-diagnostics` buffer and written to stderr using the same structured, privacy-safe representation.

The promotion path records these stages:

- `preparation_started`
- `full_state_sync_started`
- `full_state_sync_finished`, with success and elapsed milliseconds
- `completion_received`
- `completion_ignored`, when correlation rejects a stale completion
- `preparation_cancelled`
- `ready_projection_dispatched`

Fields are limited to numeric generation/transition correlation, booleans, closed outcome tokens, and elapsed time. User IDs, room IDs, homeserver URLs, access tokens, recovery material, SDK error strings, and message content are forbidden.

The SDK error remains mapped to the existing closed `sdk` failure kind. This change improves observation only; it does not add a timeout or change the admission state machine.

## Testing

Tests must prove that the dual sink receives identical structured events, that formatting contains no private strings, and that promotion success, failure, stale completion, and cancellation produce the expected stages. Existing admission-ordering and diagnostics tests must remain green.

