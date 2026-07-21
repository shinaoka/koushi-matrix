# Sync Capability Probe Isolation Design

## Status

Approved design for the matrix.org connection regression introduced by PR
#289. The implementation isolates the authenticated MSC4186 invite-list
capability probe from the authoritative desktop session while preserving the
fail-closed fallback to legacy sync.

## Problem

PR #289 added one authenticated MSC4186 invite-list request before selecting
the authoritative sync backend. The request currently runs through the same
`matrix_sdk::Client` that owns the restored desktop session.

The Matrix SDK treats `M_UNKNOWN_TOKEN` as a client-wide authentication fact.
When the optional capability endpoint returns that error and refresh cannot
recover, the SDK broadcasts `SessionChange::UnknownToken`. `AccountActor`
correctly interprets that event as an invalid authoritative session, projects
`SessionLocked`, and stops the sync actor.

The 2026-07-21 matrix.org diagnostic demonstrates this exact ordering:

```text
probe_done backend=legacy reason=invite_list_unknown
session_invalidated soft_logout=false action=lock
legacy_started
legacy_state state=stopped ever_ran=false action=stop_signal
```

The capability result is allowed to select legacy sync, but it must not mutate
or invalidate the authenticated session whose legacy `/sync` request may still
be valid. The current implementation violates that boundary.

## Goals

- Preserve the authenticated invite-list behavior check required before
  selecting SyncService.
- Ensure every probe-side HTTP, parsing, timeout, and authentication failure is
  contained inside a disposable probe client.
- Prevent the probe from broadcasting session changes, marking the
  authoritative access token expired, refreshing or rotating authoritative
  credentials, or touching the encrypted SDK store.
- Preserve `Unknown -> LegacySync` as the fail-closed backend decision.
- Prove that legacy sync can commit its first response and reach `Running`
  after an unknown probe result.

## Non-Goals

- Do not weaken `AccountActor` handling of a real
  `SessionChange::UnknownToken` from the authoritative client.
- Do not fingerprint matrix.org or any other homeserver family.
- Do not remove the invite-list contract check or select SyncService from the
  `/versions` advertisement alone.
- Do not change reducers, `CoreCommand`/`CoreEvent`, Tauri DTOs, React state, or
  persisted session formats.
- Do not patch the vendored Matrix SDK unless the public SDK API proves
  insufficient during implementation.
- Do not add retries, polling, or a second sync owner.

## Selected Approach

`koushi-sdk` creates a disposable authenticated client for the probe. The
client has no persistent store and automatic token refresh is deliberately not
enabled. It receives only an in-memory copy of the current access token plus
the existing user/device metadata for the duration of one bounded request. A
refresh token is never copied into the probe client.

The alternative of adding a request-level "suppress session invalidation"
policy to the vendored SDK would couple this product fix to SDK authentication
internals and increase upstream maintenance. A raw `reqwest` request would
duplicate Matrix authentication and typed endpoint handling outside the SDK.
The disposable SDK client keeps the typed request while providing the required
failure boundary through existing public APIs.

## Ownership and API Boundary

### Authoritative `MatrixClientSession`

The existing store-backed client remains the sole owner of product session
state, token refresh, session-change observation, encryption state, room state,
and sync. Its `SessionChange::UnknownToken` behavior is unchanged.

The session supplies a short-lived access-token copy to the adapter. The copy
remains secret-bearing in-memory data and must never be logged, included in
diagnostics, formatted through `Debug`, or returned to Core. The authoritative
refresh token is not supplied to the probe boundary.

### Disposable Probe Client

The SDK adapter builds the probe client with these invariants:

- the normalized homeserver comes from the authoritative session;
- no filesystem, SQLite, crypto, or event-cache store is configured;
- automatic refresh-token handling is not enabled;
- no session-change observer is shared with the authoritative client;
- a temporary Matrix session containing the access token and no refresh token
  is restored only into the disposable client;
- exactly one typed MSC4186 request runs under the existing end-to-end
  deadline with request retries disabled; and
- all probe-client values are dropped when the result is classified.

An expired access token therefore produces `Unknown` in the disposable client.
It does not consume or rotate a refresh token. The later authoritative legacy
sync remains responsible for ordinary SDK refresh behavior.

### Core Sync Actor

Core passes the `MatrixClientSession` adapter to the probe rather than cloning
its inner `matrix_sdk::Client`. It receives only the existing closed result:
`Supported`, `KnownIncomplete`, or `Unknown`.

Backend selection remains:

```text
/versions does not advertise MSC4186 -> LegacySync
/versions advertises MSC4186
  -> isolated invite-list probe returns Supported -> SyncService
  -> KnownIncomplete or Unknown                 -> LegacySync
```

Core does not inspect raw HTTP or authentication errors and does not gain
access to session credentials.

## Data Flow

```text
SyncActor requests backend selection
  -> authoritative session provides an in-memory access-token copy
  -> koushi-sdk builds a no-store, no-refresh disposable client
  -> disposable client restores access token plus user/device metadata only
  -> one bounded typed invite-list request runs
  -> adapter maps the response to Supported / KnownIncomplete / Unknown
  -> disposable client and copied access token are dropped
  -> Core selects SyncService or LegacySync
  -> authoritative client starts exactly one sync owner
```

No probe response cursor, list payload, room identifier, authentication error,
or credential crosses into Core.

## Error Handling

Failure to construct or authenticate the disposable client, HTTP failure,
`M_UNKNOWN_TOKEN`, malformed JSON, an omitted requested list, and deadline
expiry all fail closed without changing authoritative state:

- omitted requested list -> `KnownIncomplete`;
- every other indeterminate failure -> `Unknown`;
- both select `LegacySync`;
- the normal authoritative session observer remains active; and
- only an error produced later by the authoritative client may lock the
  session.

The diagnostic result remains a closed probe-reason token. No raw error or
homeserver identity is added.

## Verification

Implementation follows RED-GREEN-REFACTOR.

### SDK regression test

A synthetic server returns `M_UNKNOWN_TOKEN` from the MSC4186 endpoint and
exposes a refresh endpoint that must receive zero requests. The test subscribes
to session changes on the authoritative client before running the probe and
asserts:

- the result is `Unknown`;
- the authoritative session-change receiver remains empty;
- the refresh endpoint was not called;
- authoritative access and refresh tokens are unchanged; and
- the operation completes inside the existing end-to-end deadline.

The test must fail against PR #289 because its probe uses the authoritative
client and triggers the session-change channel.

### Core continuation test

A deterministic Core test advertises MSC4186, returns an indeterminate result
from the isolated probe path, then supplies a valid first legacy `/sync`
response. It asserts the backend is `LegacySync`, the first response is
committed, lifecycle becomes `Running`, and no lock/stop transition is emitted.

### Existing contract tests

Retain and run the existing success, list-omission, malformed/error, timeout,
and bounded-refresh cases, adjusted to the isolated boundary. Run focused SDK
and Core tests during implementation, then the affected crate suites and the
repository's required headless gates before completion.

Because this change affects restore and sync, an attended real-homeserver
preflight against matrix.org is required after deterministic gates pass. It is
confirmation only; the automated regression tests remain the primary proof.

## Canon Amendment

The authenticated capability-probe rule in
`docs/policies/engineering-rules.md` must state that a non-authoritative probe
cannot share session-invalidation state, access-token-expiry state, token
refresh, rotating refresh credentials, or persistent stores with the
authoritative client. Probe failure must remain a backend-selection fact, not
an authentication-state transition.

## Success Criteria

- The synthetic `M_UNKNOWN_TOKEN` regression no longer reaches the
  authoritative session-change receiver.
- No probe request attempts token refresh or mutates authoritative tokens.
- Unknown capability results still select legacy sync.
- Legacy sync commits its first response and reaches `Running` after the
  isolated probe fails.
- Genuine authoritative `UnknownToken` events still lock the session.
- No frontend, DTO, reducer, persistence-format, or vendored-SDK change is
  required.
