# Dogfood Attention and Timeline Recovery Design

## Scope

This batch closes issues #239, #238, and #235 in one merge-commit PR. Issue
#235 is already implemented on `main` by `7a8921a`; this batch only reruns its
acceptance gates and fixes regressions caused by #239 or #238. It does not
reimplement or redesign the thread-root display projection.

## Issue #239: authoritative recovery and exactly-once sending

The owner comment on issue #239 is the normative feature specification. The
implementation must preserve its three independent failure domains and their
separate RED tests.

### Timeline relay

The SDK timeline remains authoritative. On actor-inbox overflow, the active
relay generation exits and reports overflow through a lossless control path.
The actor advances its generation, aborts or retires the old relay, calls
`Timeline::subscribe()` for one authoritative snapshot and its matching live
stream, emits `ResyncRequired` followed by `InitialItems`, and starts a new
generation-tagged relay. Late batches from old generations are ignored.

Increasing queue capacity, leaving an overflowed relay permanently stopped,
or repairing transaction rows in React is prohibited.

### IME ownership

All main, thread, and edit composers share one composition-lifecycle helper.
Each `compositionstart` advances an epoch and cancels the previous deferred
end. A deferred `compositionend` may clear active state only if its captured
epoch is still current. While composition is active, the textarea DOM and IME
own the value and replacement range; parent snapshot/autosave renders do not
write the DOM value. Controlled/uncontrolled mode is never toggled per
composition.

### Exactly-once submission

A synchronous resolver guard is acquired before asynchronous key resolution.
When the resolver returns `send`, a synchronous submission guard and unique
submission ID are acquired before invoking Tauri. The ID crosses frontend,
Tauri, `CoreCommand`, and reducer boundaries. Core accepts a submission ID at
most once. Tauri returns a typed accepted/rejected result only after the
matching reducer transition is observable; it does not return an enqueue-time
snapshot. Draft text clears only after acceptance. Retry remains tied to the
existing transaction and does not create a new submission.

Admission is atomic across the manager and timeline actor. The manager first
reserves one actor-mailbox slot with a closed one-shot start permit attached.
The actor must await that permit before touching the SDK or emitting a
terminal. The manager opens it only after the matching reducer acceptance was
delivered, the active ledger was recorded, and `SubmissionAccepted` was
emitted. A mailbox failure rejects without reducer acceptance. A reducer
delivery failure drops the permit, records a bounded rejected tombstone, and
replays of that ID remain explicitly rejected without reaching the SDK.

Timeout, disconnect, lag, or submit-delivery ambiguity is `Unknown`, not
rejection. The frontend retains the submission guard, ID, target, and captured
payload, leaves the draft intact, and waits for a later accepted/terminal
observation or an explicit user retry. That retry reuses the same ID and exact
captured payload. It never allocates a replacement ID and never loops
automatically. Explicit `SubmissionRejected` is the only safe pre-acceptance
release. Browser fakes implement the same accepted/replay/terminal ledger for
main sends, replies, and thread replies.

Diagnostics expose only generation/epoch/request tokens, fixed outcomes,
counts, and durations. They never expose message bodies or Matrix identifiers.

## Issue #238: native attention completion

Existing Rust-owned notification settings and attention candidates remain the
only policy source. A native adapter plays a bundled, license-compatible sound
only for a new Rust-owned candidate when `settings.notifications.sound` is
enabled and the platform capability is available. A deterministic cooldown
coalesces bursts; candidate dedupe and cooldown are separate concepts.

Persistent badge updates consume Rust-owned badge count and capability data.
Unsupported or unknown numeric-badge capabilities do not trigger misleading
fallback indicators. Zero unread and logout clear supported badge surfaces.
Native adapter failures remain non-fatal and emit only fixed, private-data-safe
diagnostic outcomes.

## Verification and landing

Each behavior change follows RED then GREEN. Focused Rust, Tauri, TypeScript,
IPC-contract, Playwright, and local headless gates run before the full batch
review. Native macOS IME verification is reported honestly: it must be backed
by a macOS lane or attended evidence and cannot be inferred from Linux/jsdom.
The batch lands as one PR using a merge commit, not squash.
