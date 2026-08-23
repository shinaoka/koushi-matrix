# Issue #570 Task C — Room-Latest, Contract Mirrors, and QA Convergence

## Status, dependency, and review gate

This is the third and final independently mergeable task under issue #570. It
must be rebased onto merged Task A and Task B. Implementation must not start
until both predecessors are on `origin/main` and this exact document has a
recorded user-approved mandatory substitute `reviewer-flash`
`Correct-to-merge` verdict.

Task C carries the remaining room-list latest-event boundary through Rust,
Tauri, TypeScript, Browser Fake fixtures, and deterministic both-server QA. It
does not reopen Task B's canonical Activity, unread/navigation, or thread
aggregate algorithms, and it does not add a React-owned fallback ledger.

#570 closes only after Task A, Task B, and this task are merged and every issue
acceptance row is audited against their combined evidence.

## Existing boundary and defect

`matrix_sdk` already recomputes a remote room latest value from event-cache
state and normally promotes the previous candidate when the newest event is
redacted. It also returns a validated replacement event as the latest value
when an original message has an effective edit. Koushi currently projects that
value directly:

- cached conversation-activity reverse scan does not explicitly reject raw
  `unsigned.redacted_because`;
- a replacement can leak the edit event's identity/relation instead of the
  original message identity and ordering;
- `MatrixRoomLatestEventSummary`, state, Tauri, and TypeScript carry no explicit
  redaction fact for restored/stale/defensive inputs;
- `roomLatestDisplayEventId` can only reject relation events, not a typed
  redacted latest value;
- fully-read matching and Activity placeholder timestamps can consume a latest
  summary without proving it is non-redacted.

Task A's persisted pending-redaction index makes out-of-order target arrival
converge in event cache. Task B makes canonical Activity, unread, navigation,
and thread state consume the shared eligible projected window. Task C must
consume those authorities, not recreate them.

## Rust room-latest contract

Add one field through the existing mirror chain:

```rust
pub struct MatrixRoomLatestEventSummary {
    // existing fields
    pub is_redacted: bool,
}

pub struct RoomLatestEventSummary {
    // existing fields
    #[serde(default)]
    pub is_redacted: bool,
}
```

`false` is the compatibility default for old serialized snapshots. The field is
required in newly generated Rust/Tauri/TypeScript fixtures. It is a fact about
the projected original event, not a UI instruction.

### Raw redaction classification

Add one private SDK helper that inspects the raw `TimelineEvent` JSON for
`unsigned.redacted_because`. It returns only a boolean and never logs the raw
event, event ID, sender, room, or content.

Use it in both room-latest paths:

1. `matrix_room_latest_event_projection` marks a defensive/restored redacted
   remote value `is_redacted=true`, retains only the typed coarse summary when
   useful, and returns no conversation activity from it.
2. `matrix_room_cached_conversation_activity` skips every redacted candidate
   during its reverse scan and continues to the preceding eligible message or
   thread reply.

The existing Matrix latest-event engine should normally promote the preceding
candidate before Koushi sees a redacted latest. The explicit field is still
required as a fail-closed contract for restored values, races, direct fixtures,
and future SDK behavior. Do not patch the vendored latest-event selector in
Task C unless a focused behavioral RED proves its existing promotion test is
insufficient after Task A; any such scope change requires a design amendment
and re-review first.

### Original identity and effective edits

A latest `m.replace` value is not a standalone room activity. Project it as the
original target only when all of the following are proven from the local
RoomEventCache:

- the exact replacement target exists;
- the target is a message/encrypted event eligible for room conversation
  activity;
- the target is not redacted;
- the replacement is the SDK-selected valid effective edit for that target.

Do not query or rank replacement relations again. The existing
`matrix_sdk::latest_events::Builder` has already selected and validated this
replacement with `check_validity_of_replacement_events`. Task C verifies only
its exact `m.replace` target, finds that original through
`RoomEventCache::rfind_map_event_in_memory_by`, and uses the existing
`matrix_sdk_ui::timeline::TimelineItemContent::from_event(room, replacement)`
adapter for effective content—the same adapter Task A uses after validity.

The projection preserves the original target's event ID, sender, timestamp,
and original relation metadata; `relation_type`/`relation_event_id` and the
`is_threaded`/`is_reply`/`has_thread_summary`/`has_reactions`/
`content_converted` boolean facts come from the original target, never the edit.
It uses the replacement's validated effective `m.new_content` only for the
preview. It never uses the edit event's fallback `* `
body, timestamp, or event ID. This is a bounded read/projection over SDK-owned
facts, not a second edit ledger. If these named SDK seams differ after Tasks
A/B merge, stop and amend rather than hand-implementing Matrix edit validity in
Core or React.

If the original target is unavailable (including bounded-cache eviction),
redacted, or cannot be projected, fail closed to no latest display anchor and
use the preceding cached conversation activity if present. Do not paginate,
fetch the network, retry, or synthesize an identity. A focused test pins this
degraded result so a future change cannot silently invent fallback semantics.

Standalone edits, reactions, redaction events, and unsupported relations never
advance `conversation_activity`. A valid effective edit keeps the original
conversation timestamp, so editing does not reorder a room or thread.

Local ordinary send values are not redacted and keep their current
transaction/send semantics. A local `m.replace` value is transiently fail-closed:
it creates no display anchor or conversation activity. The QA assertion of
original identity waits for the remote echo, which then converges to the
original event ID with effective content and unchanged ordering. A focused
local-edit→remote-echo test pins that transition. Local redaction ordering
remains the Matrix send-queue/latest-event owner's responsibility; Task C adds
no optimistic room reordering.

### Read-marker and placeholder consumers

`read_marker_matches_latest` returns false when `latest_event.is_redacted` or
when the latest summary cannot identify an original eligible event. Therefore a
redacted defensive latest value cannot suppress unread counts or become a
fully-read fallback target.

Task C also updates `crates/koushi-core/src/runtime/activity.rs` at every
room-latest consumer boundary. `fully_read_marker_updates` routes its
RoomUnread-placeholder fallback target through `activity_latest_display_event_id`
and skips rooms whose latest value is redacted or cannot identify an original
eligible event. The helper first rejects `is_redacted`; it rejects defensive
`m.replace`/`m.annotation` summaries rather
than translating `relation_event_id`, because normalized effective edits now
already carry the original identity/relation metadata. This retires Core's
second edit-identity interpreter. Other eligible original relation metadata
(such as a thread reply) keeps the original event ID.

Activity unread-placeholder ordering uses `latest_event.timestamp_ms` only when
that summary is non-redacted; otherwise it falls back to the already filtered
`conversation_activity.timestamp_ms`, then zero. The same guarded helper owns both marker-target selection and fully-read
suppression. Canonical event Activity rows remain Task B-owned.

Core normalization copies `is_redacted` without reinterpretation. State's
privacy-safe `Debug` reports the boolean but continues to redact IDs, labels,
avatars, and previews.

## Contract mirrors and generated evidence

Carry the field through:

- `crates/koushi-sdk/src/room_projection.rs`;
- `crates/koushi-core/src/room/normalization.rs`;
- `crates/koushi-state/src/state/room.rs` and every Rust constructor/fixture;
- `apps/desktop/src-tauri/src/dto.rs` only where an explicit mirror is needed;
- `apps/desktop/src/domain/types.ts` as required `is_redacted: boolean`;
- generated contract fixtures and
  `apps/desktop/src-tauri/tests/golden/frontend_app_state.json`.

The golden must contain at least one `latest_event` with `is_redacted: true` and
one with `false`; a file-wide default-only update is not evidence. Generated
keys and TypeScript declarations must match exactly.

No snapshot schema version bump is needed because Rust deserialization defaults
the additive boolean for old persisted/test fixtures and the desktop snapshot
is generated atomically by matching Rust/TS builds. If a generated-contract
gate proves this assumption false, stop and amend.

## Frontend boundary

Change `roomLatestDisplayEventId` to return `null` for `is_redacted`, then apply
its existing relation/empty-ID checks. React may use this Rust fact to avoid a
bad display/read anchor. React must not:

- scan timeline rows for a replacement candidate;
- apply edits or redactions to Activity/unread/thread state;
- decrement counts;
- reorder rooms;
- manufacture a fallback event ID.

Focused component/domain tests install Rust-shaped before/after snapshots. They
prove a redacted latest has no display anchor, an edited original retains its
original anchor with effective preview, and a later authoritative replacement
snapshot promotes the preceding event without local repair.

## Browser Fake boundary

Browser Fake mirrors the required boolean in all room fixtures/defaults. Do not
extend `activityRows`, `createActivityStreams`, `editMessage`, or
`redactMessage` to infer the #570 fix from fake timeline messages.

Task A/B already own rendered Activity/thread transition coverage. Task C does
not add a second app-harness state machine: focused Timeline/domain tests pin the
redacted/null and edited-original display anchors, the strict golden supplies
true/false Rust-shaped room-latest values, and the both-server Core scenario
owns the authoritative edit/redact sequence. A future dedicated supplied-
snapshot harness test is deferred unless those existing proofs expose a gap.

Add a direct Browser Fake regression proving its edit/redact command changes do
not mutate an already installed Activity/latest/thread projection. That pins the
boundary: a fake command records or changes its modeled timeline response, while
only a supplied Rust-shaped snapshot/event changes product projections. Existing
unrelated Browser Fake demo fixtures may continue to build their static initial
Activity streams; no new semantic repair is added to them in this task.

## Deterministic both-server QA

Add one registered headless Core scenario,
`redact_edit_convergence`, accepted by the JS runner and Rust scenario registry.
It runs with `--server=tuwunel`, `--server=synapse`, and therefore
`--server=both`. Linux GUI remains tuwunel-only per repository policy.

The scenario uses event-driven observations, never sleeps or timing thresholds:

1. create/join one private test room with two QA users and run the existing
   timeline/navigation baseline;
2. run the registered Thread stage so canonical summary/count, receive, and
   pagination contracts are proven in the same both-server scenario; Activity
   is opened after redaction for the exact absence assertion without relying on
   server-specific notification-count behavior;
3. send a new latest message, edit it, observe the remote echo, and prove the
   room-latest projection keeps the original identity with effective edited
   preview;
4. redact that latest event and wait event-driven for a promoted non-redacted
   room latest plus an open Activity projection containing no redacted identity;
5. rely on the already merged Task A/B deterministic count-zero, replay,
   out-of-order, and restart matrices for final-candidate/root absence rather
   than duplicating those algorithms in Task C;
6. run the existing generic restore/logout cleanup only as lifecycle cleanup;
   `restore_cleanup=ok` is not claimed as new Task C resurrection evidence.

Use exact event IDs internally for correlation but emit only fixed closed tokens
and counts. Final success emits:

```text
redact_edit_convergence=ok
```

Register the token in `scripts/lib/qa-token-contract.mjs`, runner tests, Rust
scenario contract tests, and `docs/agents/qa-lanes.md`. Logs and failures must
not print room IDs, event IDs, user IDs, message bodies, raw SDK errors, or
credentials. A server capability gap is a test failure, not a silent skip.

## Verify-first RED matrix

Add tests before production wiring and record behavioral RED, not compile-only
failure. Type scaffolding/default constructors may land first solely to make the
behavior tests compile.

### SDK/Core/state

- raw redacted remote latest => typed `is_redacted=true`, no direct conversation
  activity, and reverse scan promotes valid A;
- edit B selected as latest => identity/sender/timestamp of original B,
  effective `m.new_content`, and no edit-event reordering;
- local edit value => no display anchor/activity, then remote echo converges to
  original B without reorder;
- redacted, absent, or cache-evicted edit target => no synthesized anchor and
  the explicitly documented conversation-activity fallback;
- reaction, edit, and redaction relations do not advance conversation activity;
- fully-read matching rejects redacted latest;
- RoomUnread mark-read target selection skips a redacted/unidentifiable latest
  instead of writing that event ID as the fully-read marker;
- placeholder timestamp uses valid promoted/fallback conversation activity;
- live, out-of-order redaction-before-target, pagination, and restored cache
  produce equal room summaries after Tasks A/B;
- state serde without `is_redacted` defaults false and privacy-safe Debug leaks
  no private values.

### DTO/frontend/fake

- generated golden contains true and false field examples;
- `roomLatestDisplayEventId(redacted)` returns null;
- effective edited original returns the original ID;
- authoritative A/B -> edited B -> redacted B -> empty snapshots render exactly
  as supplied;
- Browser Fake edit/redact does not optimistically repair installed
  Activity/latest/thread projections.

### QA

Before wiring the new scenario stage, its contract test is RED because the token
and registry entry are absent. After wiring, focused tuwunel and synapse runs
must each emit the exact fixed token and pass the privacy scanner. Re-run
`--server=both` as the final local evidence.

## Expected files and limits

Expected production/test surface:

- `crates/koushi-sdk/src/room_projection.rs` and focused SDK tests;
- `crates/koushi-core/src/room/normalization.rs`,
  `crates/koushi-core/src/runtime/activity.rs`, headless scenario registry,
  contracts, orchestrator/stage, and focused Core tests;
- `crates/koushi-state/src/state/room.rs` plus affected constructors/tests;
- `apps/desktop/src-tauri/src/dto.rs` and golden/generated-contract tests;
- `apps/desktop/src/domain/types.ts`, `TimelineView.tsx`, every affected
  TypeScript `RoomLatestEventSummary` fixture, and focused tests;
- `apps/desktop/src/backend/browserFakeApi.ts` and its focused boundary test;
- existing browser-headless regression matrix (no new Task C harness state machine);
- `scripts/desktop-headless-local-qa.mjs`, QA token contract/tests;
- architecture/state-machine/state-ownership/QA docs and this plan/index.

Do not modify vendor code/gitlink, Task B eligibility/activity/thread algorithms,
search semantics, timeline display-redaction policy, Tauri command registration,
or persistence schema. Do not add dependencies, generic projection frameworks,
compatibility shims, sleeps, retries, TODOs, or a frontend relation ledger.

## Full validation and merge gates

Run and record:

- focused behavioral RED then unchanged GREEN matrices above;
- SDK room projection and timeline tests;
- Core room/activity/runtime/timeline/navigation/thread/headless-contract tests;
- state room/navigation/all-targets;
- Tauri DTO/golden/generated-contract tests;
- frontend focused tests, full Vitest, typecheck, lint, build, Playwright;
- `cargo test --workspace --all-targets`, relevant wasm checks, Tauri tests;
- SDK submodule guard and unchanged Task A vendor identity;
- QA binary tests and the exact scenario on tuwunel, synapse, and both;
- rustfmt, docs/agent-doc checks, boundary/security/dependency/secret scans,
  generated-output inspection, artifact inspection, and `git diff --check`.

Then generate one exact full-diff artifact, obtain the user-approved mandatory
substitute `reviewer-flash` `Correct-to-merge`, fix/re-review every finding,
push, open a PR closing #570, wait for current-head CI 7/7, merge, verify ancestry
and issue closure, and remove only disposable build/worktree artifacts.

## Acceptance mapping

| #570 requirement | Combined evidence owner |
| --- | --- |
| Recent/Unread removal and promotion | Task B canonical replacement + Task C both-server QA |
| unread/notification/highlight and first-unread convergence | Task B shared eligibility + QA |
| redacted thread attention/latest/count | Task B authoritative aggregate + QA |
| original identity/effective edits | Tasks A/B aggregate tests + Task C local-fail-closed/remote-echo room-latest test |
| no relation/redaction reordering | Task C conversation-activity matrix |
| out-of-order/live/pagination/restart | Task A pending-redaction + Task B replay + Task C QA |
| Browser Fake parity without duplicate semantics | strict golden/focused display + no-repair tests |
| privacy and typed mirrors | Debug/token scans + DTO/golden/contracts |

## Review record

- Advisory `reviewer-flash` Round 1: not ready for mandatory review. It found
  that Core's existing `activity_latest_display_event_id` remained a second edit
  interpreter, Task C did not own the fully-read/placeholder consumer file, the
  SDK adapter was unnamed, and local-edit/cache-eviction/TypeScript-fixture
  evidence was incomplete. This revision retires relation-target translation,
  adds the redaction/timestamp guards and `runtime/activity.rs` ownership, names
  the exact SDK-selected replacement/content seams, and adds all missing RED
  rows and fixture coverage. This advisory record does not clear the mandatory
  gate.
- Advisory `reviewer-flash` Round 2: `Correct-to-seek-mandatory-review`; all five
  Round 1 findings were verified fixed against current consumers and SDK seams.
  This remains non-authorizing.
- Mandatory `reviewer-flash` Round 1: `Not correct-to-merge`. It identified the
  unguarded RoomUnread fully-read marker-target fallback, unspecified edit
  boolean provenance, and unnamed local cache target lookup. The design now
  routes marker-target selection through the guarded display-ID helper, adds a
  deterministic RED/GREEN row, derives relation/content booleans from the
  original target, and names
  `RoomEventCache::rfind_map_event_in_memory_by` explicitly.
- Mandatory `reviewer-flash` Round 2: `Correct-to-merge`. Every Round 1 fix was
  traced against the live Core/SDK/vendor seams; no new findings remained. The
  user-approved substitute name is used consistently throughout this document.

Implementation remained blocked until Task B merged and the mandatory reviewer
recorded `Correct-to-merge` for this amended document; both gates are now
satisfied.

## Implementation evidence

- Verify-first Rust RED: the new raw-latest classifier test failed behaviorally
  (`redacted_raw_latest_is_classified_without_private_diagnostics`, exit 101,
  0 passed/1 failed) while the helper still returned false. The room-list marker,
  Core RoomUnread fallback, replacement-original, and state serde/privacy tests
  were added before their production wiring and participated in the initial
  failing focused runs; compile-only errors were not counted as RED.
- GREEN Rust authority: final merged-main SDK lib 163/163; Core lib
  1,057/1,057 (8 ignored) and runtime Activity 11/11; state all-targets 769/769.
  Redacted raw/cache values fail closed, validated remote edits retain
  original identity/facts with effective replacement preview, local edits have
  no anchor, read markers/placeholder timestamps use the guarded helper, and
  legacy state defaults `is_redacted=false` with private-safe Debug.
- Contract/frontend/Fake GREEN: strict Tauri golden 1/1 with explicit true and
  false examples; focused Timeline/Fake/QA tests 211/211; final merged-main
  Vitest 1,487/1,487; browser-headless 262/262 with no App unhandled-error
  signature; typecheck/lint/build and Tauri lib 168/168 (1 ignored) GREEN. The
  Browser Fake installs required booleans and its edit/redact commands leave
  installed Activity/thread/latest state untouched.
- Complete Rust/platform gates are GREEN: workspace all-targets 2,528/2,528
  (13 ignored), Tauri lib 168/168 (1 ignored), wasm check, rustfmt, SDK submodule
  guard, docs, Tauri/domain boundaries, secret scan, and diff check.
- QA GREEN: headless Core registry/binary 131/131; runner/token contract tests
  and privacy/boundary/secret scans GREEN. The new event-driven scenario
  performs a remote edit, verifies original-ID/effective-preview room latest,
  runs the registered Thread baseline, redacts that latest event, waits for
  promoted non-redacted room latest and an Activity projection without the
  redacted identity, then emits
  `redact_edit_convergence=ok`. The unchanged scenario passed on both tuwunel
  and synapse through `--server=both`; output passed the private-data scanner.
- Exact-review follow-up: App room mark-read now uses
  `roomLatestDisplayEventId` before its existing fully-read fallback; malformed
  `unsigned` fails closed without logging and has direct coverage; defensive
  redacted latest previews are null. QA child output is privacy-validated and
  persisted before process-status/token checks so failures remain diagnosable
  without exposing output. The QA description now matches the actual
  registered Thread + edit/redact + post-redaction Activity-absence proof, while
  final-candidate/restart matrices remain explicitly Task A/B-owned. The final
  unchanged `--server=both` rerun passed on tuwunel and synapse with the exact
  convergence token and all Thread-stage tokens.
