# Verification Retry and Window Drag Design

## Problem

Two defects make the session-verification gate difficult to recover from and operate on macOS.

First, a failed verification attempt stores its closed failure kind in `VerificationGateState.failure` before returning to `AwaitingVerification`. Starting a new attempt clones that gate into `Verifying` without clearing the previous failure. The renderer displays `session.gate.failureKind` in every gate state, so a retry begins on “Verifying…” while still showing the timeout from the completed attempt.

Second, the normal desktop shell owns the only draggable titlebar. The session-verification gate is rendered by an earlier full-screen return and therefore never mounts that titlebar or its Tauri `startDragging()` behavior. With the native macOS overlay titlebar, the verification window has no usable mouse drag surface.

## Design

### Retry failure lifecycle

Treat a verification failure as the result of one completed attempt, not as permanent gate metadata. When `VerificationMethodSubmitted` accepts a supported method and transitions from `AwaitingVerification` to `Verifying`, clone the gate capabilities and account kind but clear `failure` before storing the new state.

The completed failure remains visible while the user is choosing whether to retry. Once a new attempt starts, the old failure is absent. If that attempt fails, the existing correlated failure action stores the new failure and returns to `AwaitingVerification` as before. Flow-ID correlation, supported-method validation, bootstrap restrictions, and fail-closed admission behavior do not change.

The renderer continues to project the state it receives; it does not hide stale failures with a view-only condition. This keeps serialized state, diagnostics, tests, and UI consistent.

### Verification drag surface

Add a dedicated 44-pixel drag region at the top of the session-verification gate, matching the normal shell titlebar height. It is a layout region, not an interactive control, and is separate from the centered verification content so dragging cannot consume button or input interaction.

The region carries the Tauri drag-region marker and starts native window dragging only for a primary-button press. The native call is guarded by the existing Tauri runtime check and ignores a rejected drag promise, matching the normal titlebar behavior. Browser tests receive an injectable drag operation so they do not depend on a Tauri runtime.

The drag region is present for every state rendered by `SessionVerificationGate`, including trust checking, method discovery, awaiting verification, active SAS or recovery verification, bootstrap confirmation, rejection, locking, and room preparation.

## Testing

State tests must reproduce `timeout -> AwaitingVerification -> retry -> Verifying` and assert that the retry keeps the capabilities, account kind, method, and new flow ID while clearing the old failure. Existing correlation and fail-closed tests remain green.

Renderer tests must prove that the verification drag surface is present, a primary-button press invokes the injected drag operation once, and a non-primary press does not invoke it. Existing form and duplicate-operation tests must remain green, demonstrating that the drag surface does not interfere with verification controls.

Relevant Rust state tests, desktop component tests, TypeScript checks, and formatting/lint checks must pass before the PR is opened.
