# Live Session Status and Device Naming Design

**Issue:** #369

## Goal

Give the user an authoritative, compact description of the current Matrix
session from the existing connection status, and conditionally name a newly
authenticated device when the homeserver left its display name empty.

The implementation must keep Matrix semantics in Rust, must not expose secrets,
and must build on the authoritative trust-recheck path fixed by #375.

## Decisions

### One Rust-owned session-status state machine

Add a dedicated `CurrentSessionStatusState` slice to `AppState`:

- `Idle`
- `Checking { request_id, trigger, last_known_details }`
- `Ready { request_id, details }`
- `Failed { request_id, kind, checked_at_ms, last_known_details }`

`details` is one coherent result containing:

- authoritative device display name and Device ID;
- authentication method;
- current sync-state projection;
- current-device cross-signing by the owner;
- own-identity verification;
- key-backup state;
- the resulting `Verified` or `Unverified` verdict;
- the completion timestamp.

React renders this slice and never combines independent badges into a trust
verdict. A failed refresh replaces the prior `Ready` verdict with `Failed`.
After #802, transport timeout/connectivity failure retains the prior facts in
`last_known_details`; the UI may present them as stale observations but never as
a newly successful refresh or an authentication/trust downgrade.

Opening the popover and pressing **Recheck** both dispatch the same typed
command. Correlation IDs and the session generation reject stale completion.
Only one refresh may be active at a time. After #802, `Recovery` is a core-only
trigger admitted once when accepted sync state returns from unproven to
`Running`; Tauri rejects attempts to forge it from the frontend.

### Extend, do not replace, the #375 trust path

`MatrixClientSession` gains one SDK-facing session-inspection method. It asks
the SDK for the current device and own identity, observes key-backup state, and
returns a redacted coarse result.

The existing `recheck_current_device_trust` path remains responsible for the
verification-gate lifecycle. The new status refresh may use the same SDK
primitives, but it does not promote, demote, or repair session readiness.
Opening a UI popover therefore cannot change authentication admission.

### Authentication method is durable session metadata

Add a coarse `SessionAuthenticationMethod` to `SessionInfo`:

- `Password`
- `Sso`
- `OAuth`
- `Token`
- `Unknown`

Login completion records the known method. Restored legacy session JSON uses
`Unknown` through a serde default. No token or issuer credential crosses the
state boundary.

### Platform-correct conditional device naming

After an OAuth/MAS login completes, inspect the authoritative current device.
If and only if the trimmed display name is empty, rename it to the Rust-owned
platform label:

- `Koushi on macOS`
- `Koushi on Windows`
- `Koushi on Linux`

The platform arrives through the existing `DisplayPlatform` model rather than
ad-hoc OS checks. A non-empty server or user-selected name is preserved.

The rename is a one-shot post-login operation, not a launch-time migration. A
rename failure is non-fatal: login and sync continue, a private-data-free
diagnostic records the coarse outcome, and live session status shows the
authoritative unresolved name.

Password and SSO login continue to send their existing requested device display
name. The conditional post-login repair applies to OAuth **and password**
logins: the password path was the reproduced empty-name path (#474) when the
login form sent no device name, so the same repair runs after a password login
(with the empty default now left to Rust rather than sent as "Koushi"). A
user-customized name is never rewritten in either flow.

### Account-management destination reuses discovery

The popover reads `AuthDiscoveryState::Ready.delegated.account_management_url`.
The SDK discovery result is the authority; Koushi does not special-case a
matrix.org URL in React.

If the discovered URL is absent or fails the existing safe external-navigation
policy, **Manage account and devices** opens Koushi's local account/device
settings and explains that the homeserver did not advertise an external
destination.

### Compact accessible popover

`TopBar` changes the current `role="status"` container into a keyboard- and
pointer-accessible button while preserving the live sync announcement.
The popover shows:

- homeserver;
- user ID;
- device name;
- Device ID;
- authentication method;
- sync state;
- verification verdict;
- owner cross-signing;
- own-identity verification;
- key-backup state;
- last checked time.

Actions are **Recheck**, **Copy Device ID**, **Manage account and devices**, and
the existing diagnostics entry point. Copy is limited to the Device ID.
Popover focus, Escape dismissal, outside-click dismissal, and focus return are
covered headlessly.

## Diagnostics and privacy

Use source `session_status` for:

- `opened`;
- `refresh_started` with `trigger=open|manual`;
- coarse device/identity/backup outcomes;
- `refresh_settled` with verdict and elapsed time.

Use source `oauth_device_name` for:

- `inspected` with `present|empty|failed`;
- `rename_settled` with `success|failed`.

Diagnostics contain only enumerated facts, booleans, counts, request/generation
correlation, and elapsed time. They never contain homeserver, user ID, Device
ID, display name, URL, raw SDK error, or credentials.

The local UI deliberately displays the current account identifiers, but generic
diagnostic events and `Debug` implementations redact them.

## Verification strategy

Verification follows repository discipline: reproduce each contract in a
headless test before implementation.

1. State reducer tests prove Checking, success, failure, stale correlation,
   logout reset, and coherent verdict guards.
2. SDK mock-server tests prove current-device/identity queries, cross-signing
   classification, key-backup classification, conditional rename, preservation
   of a custom name, and non-fatal rename failure.
3. Core actor/runtime tests prove command routing, generation fencing,
   diagnostic privacy, OAuth post-login naming, and failure settlement.
4. DTO/golden/type tests prove the complete Rust-to-TypeScript mirror.
5. Browser-headless tests open the popover, observe Checking, settle every
   result, retry failure, copy only Device ID, use discovered management URLs,
   fall back locally, and exercise keyboard dismissal.
6. SDK mock transport proves the OAuth-only rename contract. A local homeserver
   scenario independently proves that a fresh status check settles from
   authoritative current-device data.

No manual visual inspection is acceptance evidence.
