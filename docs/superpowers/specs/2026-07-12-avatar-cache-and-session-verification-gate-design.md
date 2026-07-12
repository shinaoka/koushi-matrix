# Avatar Cache And Session Verification Gate Design

Status: approved for implementation by the user's instruction to take issues
#241 and #191 through merge. This document supersedes the affected media-cache
assumption in the 2026-06-22 issue #118 completion design and the permissive
recovery-session contract in the 2026-07-06 session lifecycle design.

## Scope

This batch resolves two independent issues in one branch and one final pull
request:

1. #241 persists previously viewed avatar bytes in the account-key-encrypted
   Matrix SDK SQLite media store and reconstructs process-local renderable URLs
   after restart, including while offline.
2. #191 makes current-device verification a mandatory admission gate for a
   usable session. Password/OIDC authentication creates a quarantined
   provisional session; only an authoritative SDK `Verified` observation may
   promote it to `Ready`.

The issues share release and privacy verification but do not share product
state. They are implemented sequentially so #241 remains independently
reviewable before the session state-machine change.

## Root Causes

### #241

`download_avatar_thumbnail` passes `false` to Matrix SDK
`get_media_content`. In the pinned SDK, that flag disables both the media-store
read and the post-download media-store write. The resulting bytes live only in
`RenderableThumbnailCache`, which is intentionally process-local and cleared on
session teardown.

The old #118 design rejected SDK media persistence because it assumed the media
store was separate and unencrypted. That assumption is stale for the pinned SDK:
`MatrixClientStoreConfig::apply_to_builder` supplies the required account key to
`SqliteStoreConfig`; `sqlite_store_with_config_and_cache_path` clones that keyed
configuration for `SqliteMediaStore`; and the SDK media service applies its
bounded retention policy when cached content is inserted. Persisting a separate
plaintext thumbnail or returning a `file://` URL remains prohibited.

### #191

Authentication completion currently emits `LoginSucceeded`, persists the
session, starts normal sync, and projects `Ready` before current-device trust is
known. Later recovery observations can move the state to `NeedsRecovery`, but
reducers, runtime command guards, persistence helpers, and the React shell all
treat `NeedsRecovery` and `Recovering` as usable. Generic SAS primitives exist,
but the desktop transport does not expose the outgoing current-session flow and
the SDK adapter does not expose the authoritative current-device verification
state as an app-owned probe/stream.

## Rejected Approaches

- A second Koushi-owned avatar disk cache is rejected because it recreates the
  plaintext-at-rest defect removed by `75521b0` and duplicates SDK retention.
- A React-only blocking overlay is rejected because background actors and
  command routes would remain usable beneath the overlay.
- Treating recovery completion, a SAS `Done` event, or cross-signing setup UI as
  proof is rejected. Only the SDK current-device verification state is
  authoritative.
- Reusing ordinary full sync while hiding its projections is rejected. A
  provisional token must not activate room/timeline/search/send/notification
  actors or publish an active saved session.
- Adding per-command exceptions throughout React/Tauri is rejected. Admission
  is owned by the Rust state and core routing boundary.

## #241 Design

Avatar requests continue to use the single MXC-keyed path for own-profile,
room/space, and timeline-sender avatars. The account actor keeps its existing
in-flight coalescing and bounded process-local renderable cache. The SDK request
switches to cache-enabled media retrieval. On an online miss, the SDK downloads
and inserts encrypted bytes under its retention policy; on a later process, the
same keyed store serves those bytes without network, after which Koushi creates
a fresh `koushi-thumbnail://` reference.

The regression gate uses a disposable keyed store and synthetic MXC. It proves:

- the first request reaches the synthetic media endpoint;
- the first client/session is dropped and the renderable cache is cleared;
- a second client opens the same keyed store while network is unavailable;
- the second avatar request succeeds with identical bytes and no network hit;
- an uncached MXC still reports the existing network failure;
- no legacy `avatar_thumbnails` file or `file://` URL is created.

An SDK adapter test also fixes the invariant that the separated media-store
configuration retains the encryption key. Source-string assertions may guard
that structure, but the restart/offline scenario is the behavioral proof.

## #191 State Machine

The serializable session machine becomes:

```text
SignedOut
  -> Authenticating / Restoring
  -> Provisional { info, phase: CheckingTrust }
  -> AwaitingVerification { info, methods, account_kind }
  -> Verifying { info, method, flow }
  -> Ready(info)

Provisional/AwaitingVerification/Verifying
  -> Rejecting { info, reason }
  -> SignedOut

Ready
  -> Locked(info) when authoritative trust becomes non-Verified
```

The DTO must distinguish method capability without secrets:

- another verified device available for SAS;
- recovery key/passphrase available;
- genuine new identity requiring bootstrap;
- discovery pending/unknown;
- existing identity with no proof method.

Raw user IDs, device IDs, flow IDs from the SDK, recovery secrets, access
tokens, and SDK errors do not enter snapshots or diagnostics. App-owned opaque
request/flow IDs remain permitted where already used.

`is_session_ready` means exactly `SessionState::Ready`. A separate narrowly
named projection-context helper may include provisional verification states
only for the full-screen gate and request-correlated verification settles. It
must not authorize room, timeline, thread, search, composer, directory,
notifications, persistence of an active session, draft/navigation restore, or
normal sync.

## Provisional Runtime Ownership

The `AccountActor` owns the authenticated but quarantined SDK session. It starts
only the minimum SDK synchronization required to initialize crypto/account data,
discover recovery and other-device availability, and exchange to-device SAS
events. This restricted capability is internal: it does not create normal child
actors, emit room/timeline/search projections, run attention handling, or meet
the public sync-ready contract.

After password/OIDC/restore:

1. install the SDK session as provisional without persisting it as active;
2. start restricted crypto synchronization;
3. subscribe to the SDK current-device `VerificationState` before evaluating
   its current value, avoiding an observation race;
4. if `Verified`, atomically promote, persist, start normal actors/sync, and emit
   the Ready transition;
5. if `Unverified`, discover available methods and enter the gate;
6. if `Unknown` or discovery fails, remain fail closed with retry/sign-out;
7. if an existing identity has neither a verified device nor recovery, reject,
   attempt server logout, erase provisional credentials and account-local
   stores, then return to `SignedOut`.

The same verification subscription remains active after promotion. A later
non-Verified observation stops normal actors, clears views and attention state,
and enters `Locked`. Stale observations are rejected using the account/session
generation already used for actor lifecycle.

## Verification Methods

### Existing Device SAS

The SDK adapter exposes an app-owned outgoing current-user verification request
that targets an eligible existing verified device without leaking its raw
identifier into state. Core owns target selection and the handle map. Tauri and
React dispatch only opaque flow actions. The full-screen gate renders the
existing seven-emoji DTO and supports accept/start, match, mismatch, cancel,
remote rejection, timeout, and retry. A terminal SAS event triggers a fresh
authoritative trust probe; it never directly promotes the session.

### Recovery

Recovery availability is discovered before methods are rendered. The recovery
key or passphrase remains in an ephemeral secret wrapper passed directly through
the command to the SDK adapter. It never enters reducer or React state; React may
own only a boolean indicating whether input is non-empty. Recovery success also
triggers a fresh authoritative trust probe and promotes only on `Verified`.

### Genuine New Account

Absence of an existing cross-signing identity is distinct from an existing but
unprovable identity. Only the former may bootstrap cross-signing and secure
recovery. The generated recovery key is delivered through the existing secure
destination mechanism, and a Rust-owned confirmation state requires the user to
confirm it was saved before the final trust probe can promote the session. No
identity-reset command is exposed from the gate for an existing identity.

## Peer Device Policy

Current-device trust and peer-device trust are separate. Ordinary sends do not
query the gate or show a verify/send-anyway modal for unverified recipients.
The SDK remains configured to send to eligible encryption-capable, non-blocked
devices. Existing cryptographic integrity errors, key mismatches, and explicitly
blocked devices remain failures. Tests pin both halves so tightening the current
device policy cannot accidentally tighten peer sending.

## UI

React chooses the auth/full-screen verification surface from the Rust session
DTO before rendering the main shell. The gate provides only methods projected by
Rust, retry, and sign out. It provides no skip, verify-later, or identity-reset
escape. Settings retain non-blocking manual peer verification for already Ready
sessions.

Browser fakes and Tauri mocks mirror the Rust machine but cannot define alternate
semantics. User-facing strings are added to the localization catalog in English
and Japanese.

## Failure And Cleanup

- Network or SDK uncertainty remains in the gate and offers retry/sign-out.
- Cancelling a method returns to `AwaitingVerification` if another attempt is
  possible; it does not become Ready.
- Rejection and sign-out stop restricted sync, cancel verification handles,
  revoke/logout best-effort, erase persisted provisional material and local
  keyed stores, and acknowledge teardown before `SignedOut`.
- Restart during a provisional flow restores no active session. If quarantined
  credentials must be persisted solely to resume verification, they use an
  explicitly provisional key and may only recreate the gate; they are deleted
  on rejection. Prefer not persisting them when the SDK flow can be restarted.
- All diagnostics expose only state names, capability booleans, counts, and
  private-data-free failure kinds.

## Verification

RED-first tests cover every issue acceptance criterion:

- #241 online fetch, process/cache reset, offline cache hit and offline miss;
- reducer transition/guard matrix for every session state;
- password, OIDC, and restore never reach Ready without SDK Verified;
- verified restore promotion;
- restricted provisional command rejection across all normal command families;
- recovery success/invalid/cancel/retry/restart;
- two-device SAS match/mismatch/cancel/timeout/reject/retry;
- existing identity with no proof method rejects and erases local state;
- new-account bootstrap and explicit recovery-key-save confirmation;
- unknown/offline first login remains fail closed;
- later trust loss locks and clears normal projections;
- unverified peer devices do not block send or cause a normal-mode prompt;
- integrity mismatch and blocked-device failures remain enforced;
- serialization, Debug, logs, QA tokens and screenshots contain no secrets or
  Matrix identifiers.

Focused Rust reducer/core/SDK tests run before desktop DTO and browser tests.
The final gate includes workspace/core/Tauri tests, frontend unit tests,
typecheck, lint, IPC contract verification, formatting, and private-data-free
headless local-server scenarios. Native GUI evidence is supplementary only.
