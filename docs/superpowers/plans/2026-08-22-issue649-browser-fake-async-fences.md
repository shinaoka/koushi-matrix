# Issue #649 Browser Fake Async Completion Fences

## Scope

Fence every browser-fake `await Promise.resolve()` completion to the authoritative operation/request state that started it. This is one request-correlation lifecycle owner; no decomposition, API/DTO shape, generic registry/epoch, timer, fixture, Rust, or `appHarnessMain.tsx` change.

Immutable baseline: main `cd5c15b6d3a8a6aa5391e4ae848f0f2138a4c404`; parent 5,777 lines / 194,383 bytes / SHA-256 `867e5ba34c685232d663a0ebf67eea0744063043c50241c4670d748f4988b565`; test 2,783 lines / 97,906 bytes / SHA-256 `ac8dc6fd607a39de68c861d49eb26b2300c4d47c6d867315c2d0dce234e559c4`.

## Async inventory

Fourteen `await Promise.resolve()` sites exist. `loadSpaceMembers` already has a complete navigation/space/generation/operation/request fence and remains exact. Add post-await fail-closed guards to the other thirteen. The sole other async yield is `loadLinkPreviews`' 50 ms timer: current fixtures expose no pending preview and session clear removes its projected timeline target, so it has no publicly reachable stale write; it remains unchanged and is recorded for re-evaluation if a pending producer is added.

1. `probeLocalEncryptionHealth` — `local_encryption.kind === "probing"` and request ID.
2. `resetLocalData` — capture its request ID; require `local_encryption.kind === "resetting"` and that ID.
3. `setLocalUserAlias` — saving alias-update request ID.
4. `ignoreUser` — saving ignored-user request ID.
5. `unignoreUser` — saving ignored-user request ID.
6. `queryDirectory` — querying request ID.
7. `previewJoinTarget` — loading preview request ID.
8. `joinDirectoryRoom` — joining request ID.
9. `updateRoomSetting` — pending room-management request/room/operation=`settings`.
10. `moderateRoomMember` — pending request/room/operation=`moderation`.
11. `updateRoomMemberRole` — pending request/room/operation=`roles`.
12. `openActivity` — opening request ID.
13. `markActivityRead` — open activity with pending mark-read request ID.

On mismatch, return the current cloned snapshot without mutating or settling any owner. Do not require a generic `isReady()` check where the operation state itself is authoritative; session clear/replacement resets or replaces that state. Do not reset request IDs or add a second session-generation counter.

## RED proof

Use only public fake APIs.

1. For each of the thirteen unguarded methods, start the operation, synchronously `logout`, then await it. Assert no rejection and exact signed-out reset projections; before the fix, stale writes or supersession-masked state occurs.
2. Cover all six clear/replacement paths with a profile alias operation: complete OIDC, failed login, switch account, change homeserver, logout, and reset local data. For reset local data, start reset first and the alias second so reset settles before the stale alias continuation. This proves both reset mechanisms used by every guarded owner: synchronous `clearSessionViews` resets and whole-snapshot replacement.
3. Dedicated destructive race: start `resetLocalData`, synchronously complete OIDC login, then await reset; the ready replacement must remain ready.
4. Same-owner supersession: start directory query A then B without awaiting. The stale A promise must return B's current querying state rather than terminal A results; B alone may settle results.

Tests must first fail on the immutable production baseline, then pass x3 with the fix. Existing normal-success tests remain unchanged and green.

## Implementation constraints

- Inline exact guards immediately after each await; no helper abstraction, callback registry, wrapper, compatibility shim, sleep, TODO, or duplicated operation default.
- Preserve all pre-await admission, request allocation, operation payloads, normal completion bodies/order, snapshot cloning and errors.
- Keep `loadSpaceMembers` unchanged as the reference pattern.
- Source check: fourteen await sites, thirteen new guards, zero public method/signature/field/map/timer/export delta.

## Verification

Focused RED/GREEN x3, browser fake/client suites, source inventory, then full frontend/Rust/Tauri/Headless/wasm/policy matrix. Mandatory design/full-diff review, latest-main comparison, CI7/7, merge, close #649 and update #551.

## Review gate

Pre-implementation review: `reviewer-flash` `Correct-to-implement`.
