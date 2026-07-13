# HANDOFF — Issue #244 Session Verification

Date: 2026-07-13

Repository: this repository (`matrix-desktop` checkout)

Branch: `codex/issue-244-session-verification`

HEAD at handoff: `6476820 Trace SAS sender readiness`

Base at handoff: `origin/main` at `6b62ae6`

Issue: <https://github.com/shinaoka/koushi-matrix/issues/244>

## User Constraints

- Diagnose accurately before changing behavior.
- Ad hoc fixes, fixed delays, speculative retries, and one-off workarounds are prohibited.
- The user will perform real-device testing.
- Do not expose or persist device IDs, access tokens, sync tokens, room IDs, event IDs, recovery keys, passwords, or other private identifiers in diagnostics or this handoff.
- Recovery-key verification now works. The unresolved path is verification with another device using SAS/emoji.

## Current User-Visible Problem

On a fresh Koushi login, choosing **Verify with another device** remains in a verifying state, receives no emoji challenge, and eventually returns to the verification screen with `Timeout`.

Element Web/Desktop sees the new Koushi device and displays:

```text
New login. Was this you?
Koushi2
Unverified
```

This notification is not a SAS request. Its **Yes, it was me** button only dismisses the unverified-session toast; it does not cross-sign the Koushi device and does not start verification.

Element has not displayed the separate **Verification requested / Start verification** toast expected for a received `m.key.verification.request`.

## What Is Confirmed

### Koushi sender-side admission is healthy

The latest real-device run produced:

```text
[koushi] core.sas_verification stage=request_started flow_id=3
[koushi] core.sas_verification stage=recipients_resolved flow_id=3 \
  other_device_count=3 recipient_count=2 eligible_device_count=2 \
  sender_device_query_visible=true \
  sender_curve_key_present=true \
  sender_ed25519_key_present=true \
  interactive_recipient_count=2 dehydrated_recipient_count=0
[koushi] core.sas_verification stage=request_send_finished flow_id=3 \
  outcome=success initial_state=created
[koushi] core.sas_verification stage=sas_start_attempted flow_id=3 source=initial
[koushi] core.sas_verification stage=sas_start_finished flow_id=3 \
  source=initial outcome=pending
```

The restricted sync then succeeded repeatedly, but no remote transition arrived:

```text
[koushi] core.sas_verification stage=restricted_sync_succeeded flow_id=3
...
[koushi] core.sas_verification stage=timeout_fired flow_id=3
[koushi] core.sas_verification stage=settled flow_id=3 \
  terminal=failed failure_kind=timeout
```

This proves:

- the current Koushi device is visible in the post-`/keys/query` device collection;
- both Curve25519 and Ed25519 public keys are present;
- two owner-signed, non-dehydrated, key-complete recipient devices exist;
- Matrix Rust SDK request construction completed;
- the homeserver accepted the outgoing to-device request;
- no remote `ready`, `accept`, `start`, `cancel`, or completion state reached Koushi before its local timeout.

`restricted_sync_succeeded` proves only that the sync HTTP operation succeeded. It does not prove that a verification event was received.

### Element sees the new Koushi device

Element successfully fetched the current Koushi device metadata and opened the unverified-session toast. This rules out the earlier hypothesis that Element cannot see the new Koushi device at all.

The user also saw an older Koushi device ID in earlier Element logs. The current Koushi device has a different ID. Local-data deletion without server logout left stale server-side sessions, so do not use the older device's toast logs as evidence for the current session.

### Element's unverified-session toast is only an acknowledgement

Relevant code:

- `element-web/apps/web/src/toasts/UnverifiedSessionToast.tsx`

`onAccept` calls only:

```ts
DeviceListener.sharedInstance().dismissUnverifiedSessions([deviceId]);
```

Observed logs confirm only dismissal:

```text
DeviceListener: Dismissing unverified sessions
DeviceListener: Hiding unverified session toast
Removed toast with key 'unverified_session_...'
```

`No toast needed` in the same recheck refers to the current Element session's own recovery/cross-signing readiness. It does not mean that Koushi was verified.

### Element sync is intermittent but sometimes succeeds

Element logs have shown both successful long-poll syncs and transport failures:

```text
GET .../_matrix/client/v3/sync [...] 200
GET .../_matrix/client/v3/sync [...] TypeError: Failed to fetch
sync Number of consecutive failed sync requests: 1
MatrixClient sync state => RECONNECTING
GET .../_matrix/client/versions [...] 200
```

A roughly 30-second `/sync` returning 200 is a normal long poll. The immediate `Failed to fetch`, transition to `RECONNECTING`, and very slow connectivity check show intermittent browser-to-homeserver transport instability. A SAS run performed while Element is reconnecting is not a valid delivery test.

### Element crypto has no active verification request

After a run, the user evaluated:

```js
mxMatrixClientPeg.get()
  .getCrypto()
  .getVerificationRequestsToDeviceInProgress(mxMatrixClientPeg.get().getUserId())
```

and obtained:

```js
[]
```

Therefore the missing UI is not merely a hidden Element toast. No active request exists in Element's Rust crypto verification cache at the time of inspection.

### Element recipient-status probe resolved after correcting DevTools filtering

On 2026-07-13, DevTools displayed the immediate return value as
`Promise {<pending>}` and did not visibly show the later `console.log` result.
The screenshot also showed `26 hidden` and a `Custom levels` filter, so normal
log-level output was being hidden. DevTools does not necessarily rewrite the
original Promise line when it settles. This is not evidence that Rust
crypto/WASM/store is blocked.

After repeating the probe with a directly awaited result, it resolved as:

```js
{ result: "resolved", signedByOwner: true, crossSigningVerified: true }
```

The current Element session therefore satisfies the critical owner-signature
condition used by the Rust SDK own-user recipient filter. This rules out the
simple explanation that the observed Element session is excluded because it is
unsigned or not cross-signing trusted. It does not privately correlate this
session's device ID with Koushi's two redacted eligible recipients.

### Latest pre-crypto collector result: no verification event observed

After installing the in-memory pre/post-crypto collector, the user retried from
Koushi as flow 5 and read the collector directly (avoiding all DevTools log
filters):

```js
{ marker: "KOUSHI_TAP_RECORDS", sync: "SYNCING", records: [] }
```

The matching Koushi stderr confirms:

```text
stage=recipients_resolved flow_id=5 other_device_count=2 recipient_count=1 eligible_device_count=1 sender_device_query_visible=true sender_curve_key_present=true sender_ed25519_key_present=true interactive_recipient_count=1 dehydrated_recipient_count=0
stage=request_send_finished flow_id=5 outcome=success initial_state=created
stage=sas_start_finished flow_id=5 source=initial outcome=pending
```

The SDK source confirms that this outgoing event type is exactly
`m.key.verification.request`, so the collector filter would match it. Therefore
flow 5 did **not** reach the receiver-side unknown-device drop boundary; the
event was absent before Rust crypto processing. Preserve the code-confirmed
receiver-side race as a real bug path, but do not call it the cause of flow 5.

The remaining boundary is Koushi's exact outgoing recipient set versus
homeserver to-device delivery to the current Element session. Koushi had two
other devices and exactly one owner-signed recipient. A privacy-safe Element
device inventory consisting only of counts and current-device booleans can now
prove whether the current Element session was that sole recipient without
logging any device ID.

### Running Element build may not match the inspected checkout

The user reports that the Element Web/Desktop instance under test is an older,
locally customized build for Japanese search. Therefore the source under
`../element-web/node_modules/matrix-js-sdk`
must not be assumed to match the running receiver. The DevTools runtime exposes
the exact Rust crypto version via `getCrypto().getVersion()`; capture that value
and the Element app version from Help/About before using source line behavior as
field-run evidence.

After updating Element X, a verification screen appeared. This is consistent
with Koushi's request reaching Element X, but it is not yet correlated to flow
5: distinguish an incoming Koushi/emoji screen from Element X's generic
`Verify this session` screen, and capture any new Koushi SAS state-transition
lines. If flow 5 advances immediately, Koushi send and homeserver delivery are
working and the empty Element Web collector is specific to recipient selection
or that older custom receiver.

The user inspected one successful `/sync` response. It contained only stream-position and device-key-count fields, with no `to_device` section. That response may have been the empty long poll immediately following the one-shot delivery response, so it does not yet prove non-delivery.

## Code-Confirmed Alternative: Receiver Device-Cache Race

The observed ordering matches a concrete receiver-side race:

1. Koushi uploads its device and confirms it in a fresh own-user key query.
2. Koushi sends `m.key.verification.request` to the eligible signed devices.
3. Element receives the request before its independent Rust crypto store has
   refreshed the Koushi sender device, potentially in the same sync window as
   `device_lists.changed`.
4. Element processes to-device events before device-list updates.
5. Matrix Rust SDK cannot retrieve the new Koushi sender's device data yet and silently ignores the verification request.
6. Element subsequently fetches the Koushi device and shows only the **New login** toast.
7. The first verification request is not reprocessed.

Code evidence:

- Element classic `/sync` handles to-device events first:
  `element-web/node_modules/matrix-js-sdk/src/sync.ts` around line 1186.
- The same sync processes `device_lists` later:
  `element-web/node_modules/matrix-js-sdk/src/sync.ts` around line 1571.
- Matrix Rust SDK ignores an incoming verification request if sender device data is absent:
  `vendor/matrix-rust-sdk/crates/matrix-sdk-crypto/src/verification/machine.rs` around lines 368–376.
- Matrix Rust SDK also rejects requests older than 10 minutes or at least 5 minutes into the future:
  the same file around line 208.

This path exists in code, but flow 5's empty pre-crypto collector proves that
the latest field run did not reach it. It remains relevant to earlier runs only;
do not treat it as the current leading cause without a pre-crypto delivery
observation.

Do not implement a fixed delay in Koushi. It cannot prove that any remote client has consumed its device-list update. A principled resolution would likely require receiver-side retention/reprocessing after the missing device data is fetched, or a protocol/state-machine design that supplies a real readiness acknowledgement.

## Reduced Alternative: Current Element Session Is Not One of the Actual Recipients

Koushi logged two signed recipients, but diagnostics intentionally contain no
device IDs. The current Element Web/Desktop session reports
`signedByOwner: true` and `crossSigningVerified: true`, so it satisfies the
critical trust condition and is expected to be eligible. Exact membership in
the redacted outgoing recipient set is still not proven.

Element can check its current session without exposing its ID:

```js
(async () => {
  const c = mxMatrixClientPeg.get();
  const status = await c.getCrypto().getDeviceVerificationStatus(
    c.getUserId(),
    c.getDeviceId(),
  );
  return {
    signedByOwner: status?.signedByOwner,
    crossSigningVerified: status?.crossSigningVerified,
  };
})()
```

The Matrix Rust SDK own-user request filter includes distinct, non-blocked
devices signed by the owner's self-signing key. The observed `signedByOwner:
true` removes the main trust-based exclusion; only exact private correlation or
another exclusion flag remains unobserved.

## Next Diagnostic Run

### 1. Confirm Element is synchronized

In Element DevTools Console:

```js
({
  marker: "KOUSHI_SYNC_STATE",
  state: mxMatrixClientPeg.get().getSyncState(),
})
```

Proceed only when the result is `"SYNCING"`, not `"RECONNECTING"`.

### 2. Recipient trust condition confirmed

Recorded result:

```js
{ result: "resolved", signedByOwner: true, crossSigningVerified: true }
```

Continue to the delivery taps; do not repeat this probe.

### 3. Install privacy-safe pre-crypto and post-crypto taps

`ClientEvent.ToDeviceEvent` is emitted only **after**
`preprocessToDeviceMessages` returns, so the prior event listener was not a raw
receiver-boundary probe. Install a pre-crypto wrapper as well. Because this
DevTools session hides normal log output and `console.warn()` still returns
`undefined`, the tap stores observations in a bounded in-memory array and
returns status directly. It records only stage, event type, and local timestamp,
not content, transaction IDs, sender IDs, device IDs, or secrets:

```js
(() => {
  const previous = window.__koushiCryptoTap;
  if (previous) {
    previous.crypto.preprocessToDeviceMessages = previous.originalPreprocess;
    previous.client.off("toDeviceEvent", previous.postCryptoListener);
    delete window.__koushiCryptoTap;
  }

  const client = mxMatrixClientPeg.get();
  const crypto = client.getCrypto();
  if (!crypto) throw new Error("Crypto unavailable");

  const records = [];
  const record = (stage, type) => {
    records.push({ stage, type, timestamp: Date.now() });
    if (records.length > 100) records.shift();
  };
  const originalPreprocess = crypto.preprocessToDeviceMessages;
  const wrappedPreprocess = async function(events) {
    for (const event of events ?? []) {
      const type = event?.type;
      if (typeof type === "string" && type.startsWith("m.key.verification.")) {
        record("pre_crypto", type);
      }
    }
    return originalPreprocess.call(this, events);
  };

  const postCryptoListener = event => {
    const type = event.getType();
    if (type.startsWith("m.key.verification.")) {
      record("post_crypto", type);
    }
  };

  crypto.preprocessToDeviceMessages = wrappedPreprocess;
  client.on("toDeviceEvent", postCryptoListener);
  window.__koushiCryptoTap = {
    client,
    crypto,
    originalPreprocess,
    postCryptoListener,
    records,
  };
  return {
    marker: "KOUSHI_TAPS_INSTALLED",
    installed: true,
    wrapperActive: crypto.preprocessToDeviceMessages === wrappedPreprocess,
  };
})()
```

### 4. Start a genuinely new Koushi flow

Use `Retry` after the prior timeout and start **Verify with another device** once. Capture all new Koushi `core.sas_verification` lines.

### 5. Inspect Element immediately

Run this after Koushi reports `request_send_finished ... outcome=success` and
Element has had one long-poll interval to receive it:

```js
(() => {
  const c = mxMatrixClientPeg.get();
  return {
    marker: "KOUSHI_TAP_RECORDS",
    sync: c.getSyncState(),
    records: window.__koushiCryptoTap?.records ?? [],
    requests: c.getCrypto()
      .getVerificationRequestsToDeviceInProgress(c.getUserId())
      .map(r => ({
        pending: r.pending,
        phase: r.phase,
        selfVerification: r.isSelfVerification,
      })),
  };
})()
```

### 6. Remove both taps

```js
(() => {
  const tap = window.__koushiCryptoTap;
  if (!tap) return;
  tap.crypto.preprocessToDeviceMessages = tap.originalPreprocess;
  tap.client.off("toDeviceEvent", tap.postCryptoListener);
  delete window.__koushiCryptoTap;
  console.warn("KOUSHI_TAPS_REMOVED");
})()
```

### Interpretation

- A `pre_crypto` record appears
  - The homeserver delivered the request into Element JS. This is the decisive
    receiver-delivery observation.
- A `pre_crypto` record appears but no matching `post_crypto` record appears
  - Rust preprocessing blocked or omitted the event. Together with the pending
    verification behavior, investigate the Element crypto/store path before
    attributing the failure solely to unknown-device filtering.
- Both `pre_crypto` and `post_crypto` records appear, but `requests` is empty
  - Delivery and preprocessing completed, but the Rust verification machine did
    not retain a request. Unknown sender device and timestamp rejection are the
    leading code-confirmed guards.
- No `pre_crypto` record while Element remains `SYNCING`
  - This Element session did not receive the request. Recipient membership and
    homeserver to-device routing remain the next boundary. The session's trust
    condition is already confirmed (`signedByOwner: true`).
- A second request succeeds after the New login/device fetch
  - The first-request device-key propagation race is confirmed.
- Element switches to `RECONNECTING`
  - The run is inconclusive because of the independent sync transport failure.

## Element X Comparison

Element X iOS and Koushi use the same high-level Matrix Rust SDK call for own-user verification:

```rust
UserIdentity::request_verification_with_methods(vec![VerificationMethod::SasV1])
```

The Koushi vendored SDK request-generation and recipient-filter code does not materially differ from the SDK commit used by the inspected Element X version. The wire request construction is therefore not an obvious Koushi-only divergence.

Element X receiver code has another drop boundary: its FFI client processes an incoming verification event only if `SessionVerificationController` already exists. If absent at event time, the event is discarded and not replayed. Relevant files:

- `vendor/matrix-rust-sdk/bindings/matrix-sdk-ffi/src/client.rs`
- `vendor/matrix-rust-sdk/bindings/matrix-sdk-ffi/src/session_verification.rs`
- `../element-x-ios/ElementX/Sources/Services/Client/ClientProxy.swift`

This is relevant to the report that Element X showed no verification request, but no Element X receiver logs have yet proven which boundary failed.

## Recovery Key and Persistence Facts

- Recovery-key verification has worked in Koushi during this investigation.
- The recovery key plaintext cannot be fetched from the homeserver. Clients can only detect that recovery/secret storage is configured and validate a supplied key.
- Changing the recovery key is account-wide: Element creates new secret storage, re-encrypts secrets, may reset key backup, and rotates dehydration state. Do not change it merely to debug SAS delivery.
- `Reset local data` clears local persistence but does not call the Matrix server logout endpoint.
- Normal Koushi `Sign out` performs best-effort server logout and then clears local persistence.
- Local-data deletion alone can leave old server-side Koushi devices visible to Element. Those stale signed devices may be included in recipient counts even though they cannot answer.

Relevant Koushi code:

- `crates/koushi-core/src/account.rs::perform_logout`
- `crates/koushi-core/src/account.rs::clear_account_persistence`
- `crates/koushi-core/src/account.rs::handle_reset_local_data`

## Branch Changes

Issue #244 work currently consists of these commits on top of `origin/main`:

```text
264d7c6 Fix session verification admission lifecycle
8073bd6 Harden verification promotion cancellation
e92296c Add verification admission ordering coverage
878089b Cover verification proof and cancellation ordering
101fd0a Correlate promotion task ownership and surface transport errors
60cf133 Narrow projected login failure detection
434f9be Document verification preparation diagnostics
fe35bfd Trace verification room preparation
72bc7fa Start sync after verification promotion
3484047 Design SAS delivery diagnostics
1510bbc Plan SAS delivery diagnostics
ae2cf65 Trace SAS request delivery boundary
3a91cae Trace SAS verification lifecycle
b95a751 Trace SAS restricted sync delivery
82b10b3 Cover every SAS start path
3161d87 Trace SAS sender readiness
```

The latest commit adds observations only. It does not change network calls, request construction, recipient selection, send order, retries, delays, or verification behavior.

Latest diagnostic fields added to `recipients_resolved`:

- `sender_device_query_visible`
- `sender_curve_key_present`
- `sender_ed25519_key_present`
- `interactive_recipient_count`
- `dehydrated_recipient_count`

All are booleans or counts derived from the already-fetched `UserDevices` collection. No IDs are retained or logged.

Design and plan:

- `docs/superpowers/specs/2026-07-13-sas-delivery-observability-design.md`
- `docs/superpowers/plans/2026-07-13-sas-delivery-observability.md`

## Verification Already Run

For commit `3161d87`:

- `cargo test -p koushi-sdk`
  - all SDK unit/integration tests passed; 0 failures.
- `cargo test -q -p koushi-core --lib -- --test-threads=1`
  - 528 passed, 5 ignored, 0 failed.
- `cargo fmt --all -- --check`
  - passed.
- `git diff --check`
  - passed.
- Commit hooks reported the vendored Matrix SDK submodule synchronized and the staged secret scan clean.

Do not claim the SAS path is fixed. Only recovery and observability have been demonstrated; real-device SAS still times out.

## Working Tree and Process State

No Koushi, Tauri dev, Vite, or Koushi test process was intentionally left running by this agent.

At handoff time, tracked issue changes were committed. The following untracked paths existed:

```text
HANDOFF.md
apps/desktop/src/graphify-out/
docs/design/sidebar-dm-rooms-sort-mock.svg
log
```

`HANDOFF.md` was explicitly authorized by the user to be replaced. The other untracked paths belong to the user and must not be modified, staged, or deleted.

## 2026-07-13 Source Analysis: Receiver-Side Silent-Drop Path Confirmed

A follow-up source reading (Koushi vendored SDK, Element Web `matrix-js-sdk`
rust-crypto, Element X FFI, and FluffyChat `matrix-dart-sdk`) confirmed that a
first-request device-key race **can** silently discard a verification request
and identified the relevant drop boundaries. It does **not** establish that the
failed field run took this path; no new run was performed and the raw request
has not yet been observed at the Element receiver boundary.

### Confirmed drop chain (Element Web receiver)

1. `vendor/matrix-rust-sdk/crates/matrix-sdk-crypto/src/verification/machine.rs`
   lines 368–376: on an incoming `m.key.verification.request`, if
   `store.get_device(sender, from_device)` returns `None`, the machine logs
   `Could not retrieve the device data for the incoming verification request,
   ignoring it` and returns. There is no retention, no inline key query, and no
   reprocessing after the device later appears.
2. `vendor/matrix-rust-sdk/crates/matrix-sdk-crypto/src/machine/mod.rs`
   `preprocess_sync_changes` (lines ~1796–1820): `device_lists.changed` only
   marks the user for a later `/keys/query`; it does not fetch the missing
   device inline. `outgoing_requests()` creates that query after sync processing.
3. Element Web makes the ordering more explicit. In
   `element-web/node_modules/matrix-js-sdk/src/sync.ts`, to-device events are
   processed around line 1192, while `device_lists` is processed later around
   line 1571. `rust-crypto.ts::preprocessToDeviceMessages` passes an empty
   `DeviceLists` to the OlmMachine; `processDeviceLists` is a separate later
   call, and `onSyncCompleted` only then starts outgoing request processing.
   Therefore, if the Koushi sender device is not already in Element's Rust
   crypto store, a request and new-device notification delivered in the same
   sync have a structural race: the request is inspected before the missing
   device can be learned by `/keys/query`.
4. `rust-crypto.ts::onIncomingKeyVerificationRequest` calls
   `olmMachine.getVerificationRequest(...)`. If the Rust machine discarded the
   request, this returns `undefined`, Element logs `Ignoring just-received
   verification request ... which did not start a rust-side verification`, no
   `VerificationRequestReceived` is emitted, and no verification UI appears.
   The observed empty `getVerificationRequestsToDeviceInProgress()` result is
   consistent with this path, but is not unique proof of it: non-delivery, a
   different recipient session, expiry/cancellation, or inspecting after the
   flow settled can also produce an empty result.

### Element sync instability can increase the race probability

While Element is `RECONNECTING`, both the `device_lists.changed` entry and the
queued to-device request can be delivered together in a catch-up sync. For a
brand-new device that is not already cached, that raises the probability of the
ordering above. It is not deterministic: the device may already be cached, a
key query may already be in flight, the events may be split across syncs, or the
current Element session may not be a recipient. A later `/keys/query` followed
by the **New login. Was this you?** toast is consistent with the observed order,
but does not prove the request was delivered or dropped.

The timestamp guard in `machine.rs` lines 208–220 (reject if older than 10
minutes or more than 5 minutes in the future) adds two secondary failure modes: an
Element outage longer than 10 minutes rejects the request even after the
device is known, and a greater-than-5-minute clock skew between the Koushi and Element
machines rejects it with `too old or too far into the future`.

### FluffyChat demonstrates an alternative receiver design

`matrix-dart-sdk/lib/encryption/utils/key_verification.dart` lines ~385–392:
on an incoming request from an unknown device, the Dart SDK first runs
`await client.updateUserDeviceKeys(additionalUsers: {userId})` inline and only
if the device is still unknown sends an explicit
`im.fluffychat.unknown_device` cancel back to the sender. It closes exactly
the local missing-device window when the key query succeeds. The cancel code is
FluffyChat-specific, and this comparison does not prove which path the failed
Element run took or by itself establish a protocol requirement. It does show
that inline refresh plus an explicit terminal response is a viable client
design instead of immediate silent discard.

### Element X second drop boundary confirmed

`vendor/matrix-rust-sdk/bindings/matrix-sdk-ffi/src/client.rs` lines ~385–408:
the to-device verification request handler runs only when
`SessionVerificationController` already exists; otherwise no controller callback
is made, and controller creation does not replay the missed callback. The Rust
verification object may still exist internally, so this specifically confirms a
lost FFI/UI notification boundary, not necessarily deletion of the underlying
verification state. It is independent of the unknown-device drop above.

### Cheap diagnostic shortcut before the next tap run

Search existing Element console/rageshake logs for these exact strings:

- `Could not retrieve the device data for the incoming verification request`
  → if present for this flow, delivery succeeded and the device-key race
  dropped it. Rust/WASM tracing is not guaranteed to surface this line in the
  Element console, so absence is not evidence against the race.
- `too old or too far into the future`
  → timestamp rejection (clock skew or delayed catch-up delivery).
- `Ignoring just-received verification request`
  → the js-sdk layer saw the raw event but the Rust machine had no request;
  this is the most useful existing Element-side log string.

Regardless of whether these strings appear, use the privacy-safe pre-crypto and
post-crypto taps above on the next run. The pre-crypto tap distinguishes JS
receiver delivery from non-delivery without relying on Rust tracing or a
healthy OlmMachine; the post-crypto tap shows whether Rust preprocessing
returned the event.

### Fix direction (for decision, not yet implemented)

If the taps confirm receive-then-drop, the root defect is receiver-side,
so Koushi alone cannot fully fix third-party Element installs. Candidate
directions are:

1. Upstream matrix-rust-sdk contribution: on unknown sender device, either
   query keys inline (Dart SDK pattern) or retain the raw request and
   reprocess it after the next `/keys/query` completes; at minimum send an
   explicit cancel instead of failing silently.
2. Koushi product side: retain the existing explicit Retry affordance, or
   separately design a protocol-state-driven retry lifecycle if product
   requirements demand it. Do not assume that an automatic retry is principled:
   without a deterministic test and duplicate/cancellation semantics it remains
   a sender-side mitigation, not a root fix, and risks violating the user's
   prohibition on ad hoc workarounds.

## 2026-07-13 Follow-Up Review: Element X Readiness and Koushi Send Ordering

Follow-up source reading asked why Element-to-Element SAS usually succeeds and
whether Element X mitigates the unknown-sender-device drop. Static analysis
confirms readiness checks, but it does not establish a comparative success
rate or the cause of the failed field run.

### Element X narrows, but does not eliminate, its controller window

In
`../element-x-ios/ElementX/Sources/Services/Client/ClientProxy.swift`
(`updateVerificationState`, around lines 1092–1123), Element X attempts to
create `SessionVerificationController` only after the SDK verification state
leaves `unknown`. Its comment says the controller requires the user's identity,
which is unavailable before a keys-query response, and uses verification-state
updates as an approximation for that readiness.

This is a real readiness gate. It normally creates the controller before the
non-unknown verification state is projected to the UI, reducing the FFI
notification-loss window identified above. It does not replay a request that
arrived while the controller was absent, so it does not completely close that
boundary and is not evidence that Element X handles every ordering.

In
`vendor/matrix-rust-sdk/bindings/matrix-sdk-ffi/src/client.rs::get_session_verification_controller`,
controller construction requires `get_user_identity(own_user)` to return a
cached identity. This proves identity readiness, but the function does not
perform a key query and does not prove that the current device upload completed
before controller creation. In addition, matrix-sdk's
`send_outgoing_requests()` uses `buffer_unordered`, so an initial keys upload
and keys query returned by the OlmMachine may execute concurrently. The source
therefore does not justify the stronger claim that Element X guarantees an
upload-then-query round trip, or that its new device has been server-visible
for a meaningful interval before a request can be sent.

Neither Element X gate fixes the machine-level unknown-sender-device discard in
`matrix-sdk-crypto`.

### Element-to-Element timing remains a plausible hypothesis, not a source fact

All three clients ultimately use the Rust SDK's own-user verification request,
but their wrappers have different prerequisites:

- Element Web's `rust-crypto.ts::requestOwnUserVerification` obtains the cached
  own identity and then sends the request; it does not explicitly refresh the
  current device list in that method.
- Element X requires a constructible session-verification controller, as above.
- Koushi performs an explicit out-of-band own-user key query immediately before
  creating the verification request.

It is plausible that ordinary Element UI and startup timing gives healthy
receiver sessions enough time to observe `device_lists.changed` and query the
new device before a human starts verification. The inspected source does not
measure that timing or show that it is the primary reason Element-to-Element
usually succeeds. Treat this as a hypothesis requiring an instrumented
two-client run, not as a confirmed explanation. The receiver-side silent-drop
path remains present when the sender device is absent from the receiver store.

### Koushi does not send the request concurrently with its initial upload

The first Koushi password login is storeless and does not sync or initialize
encryption. After restoration into the encrypted per-account store,
`start_provisional_runtime` starts the first restricted `sync_once`. Rust
matrix-sdk calls `send_outgoing_requests()` before that `/sync`, which is where
an initial `/keys/upload` can be sent.

However, the state machine does not expose the verification choice until
`FirstRestrictedSyncFinished` succeeds and verification-method discovery
completes. More importantly,
`koushi_sdk::request_own_user_sas_verification` calls
`encryption.request_user_identity(own_user)` immediately before sending; that
API performs an explicit `/keys/query` and applies its response. Koushi then
reads `get_user_devices`, records sender/recipient diagnostics, and only then
calls `request_verification_with_methods`.

Therefore the previous claim that Koushi structurally compresses initial
`/keys/upload` and verification send into the same window is not supported by
the current code. In the failed `flow_id=3` run, the post-query local store
contained the current Koushi device and both identity keys
(`sender_device_query_visible=true`, `sender_curve_key_present=true`, and
`sender_ed25519_key_present=true`). This is strong sender-side readiness
evidence, but it does not force the independent Element receiver to refresh its
own device cache.

The live alternatives therefore remain: (a) Element received the request while
its Rust crypto store still lacked the Koushi device and silently discarded it,
(b) despite satisfying the trust condition, the observed Element session was
not one of the exact redacted recipients, (c) the event was not delivered
during the unstable sync period, or (d) a timestamp/other
receiver-side guard rejected it. The privacy-safe pre/post-crypto tap run
remains the discriminating test.

## Recommended Next Agent Decision

Do not add another Koushi retry/delay yet.

First complete the privacy-safe pre/post-crypto tap run above. Once the boundary
is known:

1. If Element receives then discards the event, reproduce the missing-device-data ordering in an SDK test and fix/reprocess at the receiver boundary, preferably upstream or in the vendored SDK with a principled lifecycle.
2. If Element never receives the event, add a private-safe proof that the observed session is in the actual outgoing recipient set, or inspect the outgoing request under a local redacted test hook. Do not log raw IDs. The trust condition is confirmed, but exact recipient correlation is not.
3. If the second request works only after the device fetch, write a deterministic test for the initial device-key propagation race before designing the fix.
4. Keep the Element `/sync` instability separate from the Koushi SAS defect. A run that enters `RECONNECTING` is inconclusive.

## 2026-07-13 Latest Field Result and Upstream Search

The latest field run changes the immediate conclusion. Koushi resolved three
other devices, selected two owner-signed/interactive recipients, sent the
request successfully, and then completed the full SAS lifecycle after one
recipient (Element X) answered:

`created -> ready -> sas_started -> started -> accepted -> sas_presented -> confirmed -> done`.

This establishes that Koushi's outgoing request, restricted sync loop, SAS
negotiation, confirmation, and completion all work end to end. The user reports
that Element X displayed the verification UI, while Element Desktop did not.

That observation alone is **not** proof of an Element Desktop bug. The Matrix
Client-Server specification explicitly defines multi-device verification this
way: the requester sends the same transaction to all eligible devices; once
one device returns `m.key.verification.ready`, the requester cancels the other
devices with `m.key.verification.cancel` code `m.accepted`. Thus Element
Desktop may legitimately show nothing (or dismiss too quickly to observe) if
Element X accepts before Desktop projects the request. A discriminating field
test must keep Element X fully offline and leave only Element Desktop online.

There is nevertheless a concrete, still-present Element/matrix-js-sdk race
candidate:

- Current `matrix-js-sdk/src/sync.ts` calls
  `preprocessToDeviceMessages(...)` around line 1192.
- It does not call `processDeviceLists(...)` for the same `/sync` response
  until around line 1573.
- Rust `matrix-sdk-crypto` handles an incoming verification request by looking
  up `from_device`; if that device is absent, it warns that it could not
  retrieve device data and returns without retaining or replaying the request.
- The upstream PR which originally wired device-list changes into Rust crypto
  (`matrix-js-sdk#3254`) introduced these as separate calls, and its merge
  comment explicitly notes that the relevant integration test quality gate was
  not satisfied because the available device-list integration tests were tied
  to legacy crypto.

An official GitHub search found no existing issue that exactly describes
"same `/sync`: new sender device-list change plus verification request, request
silently discarded". Related evidence exists: `matrix-js-sdk#4591` is an open
Major E2EE defect for stale device-list tracking, but it concerns full syncs
and is not a duplicate of this suspected ordering bug.

Runtime version clarification: before the Element update, the browser reported
`matrix-sdk-crypto 0.16.0`, crypto-wasm git SHA `1ac734e`, Vodozemac `0.9.0`.
Official history maps `1ac734e` to crypto-wasm v18.0.0. The local current
Element checkout uses crypto-wasm v18.2.0 (`65b585f`) but still reports the
same Rust crate and Vodozemac versions. The v18.0.0-to-v18.2.0 verification
machine diff is only a Clippy test annotation; no receiver-drop fix was shipped
there. Therefore "old Rust crypto version" is not the explanation.

### Next exact test

1. Force-quit Element X and ensure it cannot run in the background.
2. Reinstall the in-memory pre/post-crypto tap in the newly updated Element
   Desktop (an update/reload removes the previous monkey-patch).
3. Retry Koushi verification without interacting with any other device.
4. Record the Koushi terminal lifecycle and the Element tap records.

Interpretation:

- raw pre-crypto request present, post-crypto absent: confirms the
  Element/Rust device-list ordering/drop boundary;
- neither present while Koushi identifies Desktop as a recipient: delivery or
  recipient-correlation boundary;
- both present but no UI: Element UI subscription/projection defect;
- Desktop completes SAS: prior "X only" behavior was the specified
  first-accepting-device-wins flow, not a Desktop defect.
