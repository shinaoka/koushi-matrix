# Session Trust Discovery Timeout Design

## Problem

When a stored Matrix session is restored, Koushi installs it provisionally and waits for authoritative current-device trust plus the available proof methods. A signed release build reproduced a permanent-looking “Checking device trust…” screen after macOS requested access to existing Keychain items. Signing out and logging in again with a recovery key succeeded, which isolates the failure to the stored-session restoration gate rather than signing, notarization, the encrypted SDK stores, or recovery itself.

The provisional verification-method task currently awaits `discover_current_session_verification_methods` without a client-side deadline. That SDK operation awaits identity and device queries. If either future remains pending, no `VerificationMethodsDiscovered` message reaches the account actor, the reducer remains in `ProvisionalPhase::DiscoveringMethods`, and the UI offers only Sign out. The UI also renders every provisional phase as “Checking device trust…”, concealing whether it is checking trust, discovering proof methods, or reporting a failed recheck.

## Scope

This change must:

- bound verification-method discovery with a client-side timeout;
- project timeout and SDK failure into the existing retryable provisional failure state;
- distinguish checking trust, discovering methods, and failed recheck copy;
- expose Retry while discovery is pending and after it fails;
- record privacy-safe discovery lifecycle diagnostics to the bounded diagnostics buffer and stderr;
- preserve generation and serial correlation so cancelled or stale discoveries cannot mutate the active session.

This change does not suppress macOS Keychain authorization prompts, migrate Keychain ACLs, change recovery/SAS protocol behavior, delete stored data, or bypass the verification gate.

## Architecture

The timeout belongs at the `koushi-core` actor boundary. `AccountActor::discover_verification_methods` owns the spawned task and already owns its cancellation and serial correlation, so it wraps the complete SDK discovery future with `executor::timeout`. This keeps the SDK API focused on Matrix facts and gives the actor a single bounded lifecycle for all underlying requests.

The task sends a typed result instead of an unconditional gate:

- success carries `VerificationGateState`;
- timeout carries `VerificationGateFailureKind::Timeout`;
- SDK/unusable discovery carries `VerificationGateFailureKind::Sdk`.

The actor accepts the result only when generation, serial, and active-session guards match. A failure dispatches a dedicated reducer action rather than fabricating an “unknown” gate. The reducer moves `ProvisionalPhase::DiscoveringMethods` to `ProvisionalPhase::RecheckingTrust { failure }`, retaining the session and exposing an explicit Retry path.

Retry cancels the owned discovery task, increments the discovery serial, and starts a fresh bounded discovery for the current generation. Any completion already queued from the cancelled task is rejected by the existing correlation guards.

## UI State

The verification gate renders phase-specific text:

- `checkingTrust`: “Checking device trust…”
- `discoveringMethods`: “Discovering verification methods…”
- `recheckingTrust`: the finishing/retry state plus its closed failure label

Retry is available during `discoveringMethods` and `recheckingTrust`. It is not shown during the initial synchronous `checkingTrust` projection because the authoritative trust observation is already delivered through the actor mailbox and has a separate restricted-sync deadline.

The Retry command must restart method discovery when the current phase is `discoveringMethods` or a failed recheck. It must not merely re-read the same current trust value, because an unchanged `Unverified` observation would otherwise leave the pending discovery task untouched.

## Diagnostics

Add a `core.verification_method_discovery` lifecycle using the existing dual diagnostics sink. Record:

- `started` with generation and serial;
- `finished` with generation, serial, closed outcome token, and elapsed milliseconds;
- `cancelled` when the actor replaces or tears down an owned discovery task;
- `completion_received` before correlation;
- `completion_ignored` when generation or serial is stale;
- `failure_projected` with the closed failure kind.

Diagnostic fields must contain only numeric correlation values, elapsed time, and closed tokens. User IDs, device IDs, homeserver URLs, SDK error strings, access tokens, recovery material, Keychain values, and message content are forbidden.

## Failure Semantics

Use a fixed client-side timeout constant in `koushi-core`, separate from the restricted-sync server timeout. Timeout maps to the existing `timeout` gate failure. An SDK result that cannot provide authoritative method facts maps to `sdk`. Cancellation caused by replacement or session teardown is silent in UI but recorded diagnostically.

No failure promotes the session, signs the user out, or deletes local state. The user can Retry or Sign out.

## Testing

Tests must be written before production changes and must prove:

1. a pending discovery future reaches the timeout path and produces the retryable failure projection;
2. a successful discovery before the deadline preserves the existing awaiting-verification transition;
3. replacement cancels the old task and stale completion cannot alter the current generation/serial;
4. reducer failure handling changes only `DiscoveringMethods` into retryable `RecheckingTrust`;
5. Retry restarts discovery rather than only re-projecting unchanged trust;
6. the UI renders distinct copy and Retry availability for each provisional phase;
7. diagnostics contain the expected lifecycle stages and contain no private strings.

Existing recovery, SAS, promotion, session teardown, and state-transport tests must remain green.
