# Remaining issues: one PR through merge

## Contract

Address #858, #856, #855, #850, #847, #839, and #838 in one final PR.
The maintainer explicitly extracted the viewport-library selection/integration/
qualification into #859 and closed the former #846 umbrella. Do not silently
reintroduce that independent scope or claim its qualification is complete.
Baseline: main `a44040ce0e` (merged #857). Preserve unrelated local changes.

Consulted canon: REPOSITORY_RULES.md; architecture overview, state-machine and
i18n; engineering-rules; agent environment, verification and plan index.
Rust retains product semantics; browser fixtures supply explicit snapshots, not
an alternate state machine. Reproduce before production edits. No independent
AI review or delegation was requested. Run local gates and self-review before
the one PR; verify required exact-head CI, approvals and merge before closure.

## Completed implementation slices (not issue/goal closure)

### #858 Diagnostics accessibility

`components/dialogs.tsx` now interpolates the existing diagnostics title in
`action.close`. New `DiagnosticDialog.test.tsx` locates the actual accessible
button in English and Japanese and checks dismissal.
- Initial test environment error was corrected before behavioral verification.
- Behavioral RED: both locales exposed literal `{title}` instead of the title.
- GREEN: unchanged tests 2/2. Frontend typecheck passed.
- No speculative new translation validation framework or runtime logger.

### #856 Mention trigger in prose

`app/uiShared.ts` detects the last editable @ query without requiring whitespace;
existing mention atoms terminate queries. Completed ASCII email-shaped tokens
(with a dotted domain) are excluded from autocomplete. Partial text like
`hello@member` intentionally offers completion: it is indistinguishable from a
partial email, and users can dismiss or continue typing. Merely typing does not
insert a semantic mention; existing explicit selection behavior stays intact.
No Rust candidate filtering/ranking or Matrix payload logic moved to JavaScript.

`app/mentionQuery.test.ts` and `components/composer.mid-sentence.test.tsx`
cover Japanese/Latin/punctuation/newline/UTF-16 boundaries, email exclusion,
main and thread insertion into existing prose, prefix/suffix retention, caret
position and native IME confirmation.
- RED: 10 failures / 6 passes before production change.
- GREEN: unchanged 16/16.
- Existing composer, IME primitive and document suites: 81/81.
- IME checker unit tests: 4/4; production surface inventory passed.

### #850 Thread admission versus subscription

`App.tsx` accepts the matching Rust `opening` as well as `open` identity for
showing the panel, without claiming subscription is complete. `rightPanel.tsx`
announces the existing localized loading text and marks the content busy while
opening; the existing Rust-state edit guard keeps the composer disabled.
`e2e/thread-opening.spec.ts` supplies an admitted Opening snapshot and later
explicit Open/Closed snapshots; it never simulates Core subscription semantics.
- RED: both cases failed because the single click never displayed the panel.
- GREEN: 2/2, including loading, disabled composer, later enable/close and only
  one open command. No new waiter, timeout, retry, or renderer state machine.

### #855 Inline math baseline

`styles.css` uses inline-flex rather than inline-block for the scrollable inline
math box, exporting the formula baseline rather than the overflow box bottom.
Removed the fixed vertical offset. Block math retains its block layout.
`e2e/inline-math-baseline.spec.ts` renders through the actual app timeline and
KaTeX. It measures inner/outer baseline markers for x, subscript, superscript,
fraction and minimization at 14px and 20px, and tests constrained-width horizontal
scrolling for long inline and block formulas after fonts load.
- RED: baseline discrepancy 3.328125px (required <0.5px).
- GREEN: same browser test passed, including both long-form scrolling checks.
- Native macOS WebView confirmation is still required; Linux Chromium is not
  evidence for that acceptance item.

Latest combined frontend typecheck and lint (ESLint, IME inventory, agents docs,
semantic ownership guard) passed after the four slices. Full Vitest then passed
1,290/1,290 across 110 files. Full browser suite/build and final diff review are
not yet run. No PR is open for this batch.

## #847 recovery design / canon approval

The main frontier agent approves the state-machine amendment above before code:
retain one actor-local pending room-cache refill bit on accepted SDK Clear; drain
it through existing backward pagination only when pagination and causal gap work
are idle. Coalesce repeated clears, preserve SDK accumulator ownership, expose
ordinary failure without adding retries, and discard pending work on cancellation
or actor retirement. No SDK fork change, persisted recovery state, timers, or UI
repair. An actual SDK+Core regression reaches `EndReached`, applies ignored-user
account data, consumes Clear, and fails specifically at `ignore-refill` before
the change. Both ignore and unignore must pass without incoming events/viewport
requests. After implementation, the unchanged test passes both cycles. Full
Core library: 1,012 passed / 9 existing ignored. The actor retains one pending
bit, checks existing pagination/gap ownership before scheduling, and reuses the
normal page worker and its typed result. No SDK changes. Local-homeserver
verification and final cross-boundary audit remain pending.
Rust iteration reuses the existing main target with line-tables-only dev debug
information and CARGO_BUILD_JOBS=4; these are functional tests, not performance
measurements.

## #839 acceptance audit checkpoint

Merged #857 already bounds the avatar actor to six active downloads plus 256
pending URIs (`account/profile.rs`), retains per-request waiters, cancels the last
consumer, and rejects canceled completions. Existing real MatrixMockServer tests
exercise capacity and cancellation. `domain/avatarThumbnails.ts` now eagerly
considers only the own avatar, not all room/Space/invite icons; list icons report
viewport demand. Full receipt-reader windows, compact caps and keyboard support
have merged tests and historical Tuwunel/Synapse live-signals evidence in the
scoped-reader worklog (lines 1190–1284).

Do not treat this as full #839 closure. App still has a URI-keyed reference-count
registry; generic avatars use download/cancel request identity rather than the
issue's explicit account/surface/generation observation contract. People rows
still render initials (`PeoplePanel.tsx`, avatar=null). Audit those remaining
contract/surface requirements before changing ownership, and obtain the requested
actual bounded-avatar request evidence on disposable servers (live-signals pass
alone does not prove avatar request counts). No code changes for #839 yet.

### #839 remaining architecture proposal

Prepared [the shared-demand completion proposal](../specs/2026-09-09-issue839-avatar-demand-completion.md)
from the existing scope registry, account downloader, receipt resource and profile
invalidation sources. It records newly inspected Element Web/X iOS lifecycle
behavior with exact revisions, reuse/retirement boundaries, explicit bounds and
pre-GUI local-server evidence. It is a proposal awaiting canon-change approval,
not an implemented or verified shared-demand contract. No new state framework or
second downloader has been introduced.

### #839 same-session replacement race

Found a separate concrete bug in the existing shared downloader: after the final
waiter cancels, a new demand for the same MXC may be present when the canceled
task's queued completion arrives. Session generation and MXC equality cannot
distinguish the old task from the replacement. An actor/MatrixMockServer regression
was runtime-RED: an injected old Ready result incorrectly settled the replacement
whose actual HTTP fetch should fail. Amended the Profiles And Avatars canon before
fixing it. AvatarFetched now carries the existing Tokio task identity and the
actor compares it to the current abort handle before touching waiters, counters
or cache. No extra counter/registry was introduced. Regression assertions are
unchanged (the new internal message field supplies an obsolete task identity).
All five account profile actor tests passed, including capacity, cancellation,
cache reuse and retired-session rejection. Logs:
`/tmp/koushi-avatar-replacement-red.log`, `/tmp/koushi-avatar-replacement-green.log`.
This fixes one required stale-result boundary, not the remaining scope/demand
migration or the 1,500-target local-server evidence.

## #838 upstream comparison and Phase A start

Inspected Element Web `1b06092990a28edca91a90bbd904acb523a74ba3`,
`apps/web/src/components/views/dialogs/CreateRoomDialog.tsx:138–150,202–225`:
public creation extracts the local part from the full alias and validates the
alias field before creating. Inspected Element X iOS
`ac183daff8ffd64158fb5712f67daeacc68c1ba5`,
`ElementX/Sources/Screens/CreateRoomScreen/CreateRoomScreenViewModel.swift:97–110,141–175`:
room-name changes suggest an alias until manual editing disables synchronization;
visibility/name reset may reenable it. Its RoomDetailsScreen/JoinedRoomProxy
sharing path delegates to the Rust SDK matrixToPermalink API (source search;
read the complete methods before integration).

Koushi will keep manual address edits across later name/visibility changes rather
than reset them, following #838. Rust owns the suggestion/validation and submitted
alias; the GUI retains only raw unsent drafts and renders returned preview/status.
Use the SDK's room permalink API rather than building links in React. Availability
must remain unproven until a server response; preserve drafts on collision.

The main frontier agent approves the stateless preview boundary documented in
architecture/overview.md before implementation: Core borrows only the Ready
session, SDK/Ruma validates against the Matrix user-ID server (not the HTTP
homeserver URL), and creation reuses validation. GUI preview responses are
presentation of unsent drafts, not a new product state machine or availability
claim. A typed redacted preview DTO avoids new AppState fields/commands solely
for pure draft inspection. Room creation still uses the existing authoritative
CoreCommand/CoreEvent path.

Phase A started with `koushi-state::suggest_room_alias_localpart`: Unicode
letter/number segments become a lowercase hyphen-separated editable local part;
Japanese is preserved and unsuitable names yield empty. A focused integration
test was compile-RED before the helper, then GREEN. SDK preview validation now
returns a redacted typed `RoomAddressPreview` and validates a full alias against
the Matrix user-ID server, including ports, empty/manual/invalid input, and the
255-byte creation limit (Ruma's enabled arbitrary-length compatibility otherwise
accepts the oversized test). SDK preview tests: compile-RED then 3/3 GREEN;
full SDK lib 143/143. `create_room` reuses validation before any public-room
network request. CoreConnection exposes a borrowed-session-only preview method;
its test was compile-RED then GREEN, proving delegated HTTP hosts do not replace
the Matrix server, manual drafts survive name changes, generation is unchanged,
and signed-out previews are rejected.

No Tauri/GUI preview wiring exists yet. Collision mapping, sharing DTO, both-server
proof and GUI remain. Inspected pinned SDK `Room::matrix_to_permalink`: canonical
alias, otherwise alternate alias, otherwise room ID with SDK-computed via servers.
Use this method exactly rather than inventing a room alias or routing server.

Sharing checkpoint: the public RoomSettingsSnapshot already had `share_link` and
GUI consumers; no second public DTO field was needed. Replaced the existing
state-layer manual percent-encoding/alias selection helper with the SDK
`Room::matrix_to_permalink()` result in the SDK snapshot and direct Core mapping.
Removed the unused state helper and moved its alias-selection coverage to an
actual SDK synced-room test, covering canonical/alternate aliases, canonical
changes and alias removal/room-ID fallback. This test was compile-RED on the
missing SDK field, then GREEN. SDK lib 144/144, Core mapping 1/1, state room
management 16/16 passed. No both-server share-link proof or GUI acceptance claim
is made yet; those checks remain.

Collision checkpoint: approved the closed `RoomFailureKind::AliasInUse` addition
in architecture/overview.md before implementation. A real SDK HTTP createRoom
request against MatrixMockServer returning `M_ROOM_IN_USE` reproduced the defect:
it was classified `http`, not `alias_in_use`. The unchanged test now passes and
checks raw server text is absent from the exported error. SDK maps the Matrix
error code (not human-readable text) and Core retains it in the serialized
CoreFailure. Core room operation tests 13/13 passed. Generic non-creation operation
projection maps this kind to Invalid; mention operations retain their coarse SDK
failure. Transport/UI actionable inline rendering and actual local-homeserver
collision evidence are still outstanding at this checkpoint.

Local-server checkpoint: extended the existing `directory` stage (retaining its
previous command coverage) with ordinary Core CreateRoom using the Rust suggestion,
canonical-address equality, Ruma parsing of the SDK share URL, a second client's
join using that URL's alias, and repeated creation expecting AliasInUse. Tuwunel
passed initially. Synapse exposed an observation race: RoomCreated/list insertion
preceded canonical-alias sync. The lane now observes missing alias state with six
500ms-spaced settings requests under one EVENT_TIMEOUT; a present wrong alias still
fails. Synapse then passed, including both new evidence tokens and restored-session
cleanup. Logs: `/tmp/koushi-batch-address-tuwunel.log`,
`/tmp/koushi-batch-address-synapse-2.log`; the first failed Synapse log is retained
locally. Tuwunel predates the bounded-observation adjustment and will be covered
again by the final gate. GUI/IME/localized feedback remains unimplemented.

Phase B transport checkpoint: registered `preview_room_address` in Tauri, delegating
to CoreConnection without parsing or generating aliases in the frontend. Added the
matching RoomAddressPreview TS mirror and DesktopApi method. The frontend transport
test was runtime-RED (missing method), then all 28 client tests passed; TypeScript
typecheck and Tauri cargo check passed. Dialog state/rendering and collision error
presentation are the next step at that checkpoint.

Phase B dialog checkpoint: App now requests Rust previews for unsent drafts,
keeps a manually edited alias across name and visibility changes, and submits
that displayed local part through the existing CreateRoom command. A small
presentation hook rejects superseded draft/account responses. Public submission
requires a matching valid Rust preview; private creation remains independent.
The dialog renders a visible label, full-address preview, localized explanation
and invalid/empty status in English/Japanese, plus optional official Matrix help
through the existing external-URL opener. Removed the private-visibility handler's
alias reset. Hook test was missing-module RED, then GREEN; dialog tests were run
against the pre-change component (2 RED), then the same tests passed. Focused
hook/dialog/App suites: 114 passed; typecheck passed. Lint passed before the
subsequent optional-help-link addition and remains to rerun on final state.
End-to-end App editing/IME tests and typed collision inline handling remain; this
is not full #838 completion.

Collision UI checkpoint: Tauri now preserves AliasInUse as a structured rejection
and retains generic failure messages separately. Its serialized-kind test was
RED then GREEN. App renders localized inline collision feedback, keeps drafts,
and fences outcomes from closed/reopened dialogs by renderer lifetime; editing
the attempted alias hides the old collision feedback. Component locale tests
were RED then GREEN; focused component/App tests 87 passed, typecheck passed.
A real-App Playwright test passes for auto suggestion, manual/name/visibility
changes, exact full preview, collision draft preservation and successful retry.
It also verifies candidate-confirmation Enter does not submit the public address
form. The synthetic IME sequence must execute keydown and implicit submit in one
browser task, matching the existing zero-delay form fence; separate Playwright
round trips incorrectly expire that fence. Final focused Playwright and lint
passed. Room-info UI copy feedback/alias/no-alias checks and broader final gates
remain at that checkpoint.

Room-info checkpoint: existing UI only had a copy button and silently discarded
clipboard failures. Added visible Rust-projected full alias and share URL, plus
localized success/failure feedback; clipboard failures never claim success.
Copy completion is fenced to the current room/link and component lifetime. The
existing component test was extended and failed before the fix, then passed.
English/Japanese copy-success and copy-failure coverage now passes as part of
22 RoomInfoPanel tests. Typecheck/lint passed for the production change; subsequent
locale test expansion also passed. SDK alias-change/no-alias coverage and local
public sharing join evidence are already recorded above. Additional UI alias-change/
no-alias lifecycle coverage, whole frontend suite and final gates remain at that checkpoint.

Follow-up: both locales now exercise changed canonical aliases, a Rust-projected
room-ID/via URL without an alias, clipboard failures, and rejection of a late copy
completion after the link changes. All 22 RoomInfoPanel tests passed; clipboard
mock state is restored after each test. Whole Vitest run: 1294 passed, one failed
because the required DesktopApi migration table omitted previewRoomAddress.
Classified it as a pure typed-value method in the canonical migration map; the
unchanged three-test contract suite then passed. Typecheck, lint and production
build passed (existing large-chunk warning). Full final gates remain; no all-green
whole-suite run is claimed for the prior failing invocation.

## #847 cancellation verification checkpoint

Extended the actual SDK/Core ignored-reset fixture with explicit pagination
cancellation while a controlled HTTP messages response is delayed. It waits for
the real request, cancels, observes the matching Idle event, and checks that no
replacement HTTP request appears after the delayed response's completion window.
The first fixture attempt had no prev_batch token and never made a messages
request; that was a test setup failure, not evidence of a product regression.
Adding a synthetic prev_batch makes both recovery and cancellation exercise real
pagination. Both focused tests pass. Fresh Core lib gate after the avatar task
identity fix and this coverage: 1015 passed, 9 existing ignored, zero failures
(`/tmp/koushi-batch-core-latest.log`). Local homeserver ignore/unignore and refill
failure/coalescing coverage remain outstanding at that checkpoint.

Refill-failure follow-up: added a controlled HTTP 403 response after SDK cache
reset, checked the typed pagination Failed event, and checked that exactly one
messages request was made through a 300ms post-failure observation interval.
All three ignored-reset tests pass (`/tmp/koushi-ignore-failure.log`): recovery,
explicit cancellation, and non-retryable failure. This bounded negative check
is not a claim about every future retry trigger; explicit new user demand is
still permitted. Local-homeserver ignore/unignore and coalescing coverage remain at that checkpoint.

Local-homeserver follow-up: extended `live_signals` to issue real IgnoreUser then
UnignoreUser commands, observe the matching room's Clear/reset, and wait for an
existing visible event to return and authoritative ignored-user state to settle.
The verification sends no messages, viewport commands or restart between the
ignore/unignore commands. Tuwunel and Synapse both passed with
`ignored_user_history_recovery=ok`, their existing live-signals/navigation checks,
and restore cleanup. Logs: `/tmp/koushi-ignore-tuwunel.log` and
`/tmp/koushi-ignore-synapse.log`. Coalesced repeated-clear coverage and final
submitted-state gates still remain; neither server pass completes the full goal.

## QA token enforcement follow-up

The new directory and ignore-recovery checks initially emitted success tokens
without adding Node runner requirements. Added the normative docs/qa contract and
required-token registration for `directory`, `live_signals`, and `all`. Five
missing-checkpoint assertions were RED before registration; all 39 focused token/
runner tests passed afterward. A fresh complete frontend run now passes all
1300 tests across 112 files (`/tmp/koushi-batch-frontend-latest.log`). This supersedes
the earlier failing full-run result, but is not proof of the unfinished #839
scope migration or final PR/merge gates.

## Browser regression checkpoint while #839 approval is pending

Ran the changed real-App Playwright scenarios together on the current batch:
thread Opening→Open/Closed, inline math baseline/overflow, and public room address
manual editing/collision/retry/IME. All four passed with one worker
(`/tmp/koushi-batch-browser-checkpoint.log`). This is Chromium evidence, not the
required macOS WebView evidence. The proposed #839 contract remains unapproved
and unimplemented; this check does not authorize its implementation.

## Additional Rust gates while #839 approval is pending

Completed previously unrun full package checks on the existing implementation:

- Tauri `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib`:
  136 passed, zero failed/ignored (`/tmp/koushi-batch-tauri-full.log`).
- `cargo test -p koushi-state -p koushi-protocol`: 844 passed across 53 test
  suite summaries, including doctests, zero failed/ignored
  (`/tmp/koushi-batch-state-protocol-full.log`).
- `cargo test -p koushi-sdk`: 229 passed across 10 suite summaries, zero
  failed/ignored (`/tmp/koushi-batch-sdk-full.log`).

All commands exited zero within the explicit 120-second bounds. These are
functional checks, not overhead benchmarks. The #839 proposal has not been
approved or implemented; the goal is still incomplete and these results are not
final submitted-SHA CI or native macOS evidence.

## Boundary and privacy gate checkpoint

SDK gitlink, protocol/QA boundary, Tauri adapter boundary, command snapshot
contract and domain platform-dependency checks passed. The leaf-crate gate found
that the new `koushi-core-testkit/tests/room_address.rs` was missing from its exact
integration-target inventory. Added only that target; all eight boundary-checker
tests and the unchanged leaf boundary gate then passed. Diagnostic test isolation
also exited zero. The repository secret scanner passed for tracked files.
Logs: `/tmp/koushi-batch-boundary-gates.log` (initial inventory failure),
`/tmp/koushi-batch-boundary-gates-followup.log` (correction and remaining checks).
This inventory correction does not implement or approve the #839 proposal.

## #847 repeated-reset cancellation coverage

Added `repeated_cache_resets_share_inflight_refill_and_cancel_pending_demand`.
A real SDK ignore-list reset starts a delayed `/messages` refill. Three more
Clears pass through the existing acknowledged actor diff test seam and the normal
`handle_diff_batch` path. The actual server request count remains one; explicit
cancel produces correlated Idle, and the count still remains one after the delayed
response interval. No production code or new test API was added.

The first fixture tried to use repeated display Clears as synchronization while
the timeline was already empty and timed out. It is a fixture failure, not a
product regression. The corrected fixture uses acknowledged actor delivery rather
than assuming an empty display must emit another Clear. This is actor-level
coalescing/cancellation evidence; only the first Clear originates in the real SDK.

All four ignored-reset tests passed; formatting and Rust test-structure checks
also passed. Logs: `/tmp/koushi-ignore-coalesced.log` (initial fixture failure),
`/tmp/koushi-ignore-coalesced-corrected.log`, `/tmp/koushi-ignore-reset-suite.log`.
The overall goal and #839 approval boundary remain open.

## #838 acceptance audit and private-room browser coverage

Re-read the live #838 body, including the appended Room info requirements.
The requirement/evidence mapping is:

| Requirement | Existing implementation and evidence |
| --- | --- |
| Editable suggestion, Japanese/unsuitable names | Rust suggestion and SDK preview tests in `koushi-state/tests/room_address.rs` and `koushi-sdk/tests/room_address.rs`; actual-App public-address browser test |
| Exact account-server preview, no duplicate sigils/suffix | SDK preview tests include account server with port, full/invalid alias rejection; both-server `directory` QA compares created canonical alias with Rust preview |
| Manual edits survive name/rerender/visibility changes | Rust-preview hook freshness test, English/Japanese dialog tests, actual-App public-address browser test |
| Actionable invalid/empty/collision feedback, retained drafts | Rust typed errors, English/Japanese catalog and dialog checks, actual createRoom `M_ROOM_IN_USE` test, both-server collision QA, browser collision/retry check |
| Non-public creation does not require an alias | Existing SDK private create-request test plus the new actual-App browser test described below |
| Sufficient inline help and optional official link | English/Japanese `dialog.roomAddressHelp` explicitly explains localpart syntax and server-confirmed availability; dialog tests assert the official link |
| Share current canonical/alternate/room-ID fallback URL without changing access | SDK permalink transition test; Core direct mapping; RoomInfoPanel renders DTO and copies through clipboard without a visibility command |
| Localized copy success/failure; changed aliases and obsolete completions | English/Japanese RoomInfoPanel share/copy tests |
| Participation URL reaches created public room | Both-server `directory` QA parses the SDK URL and joins a second client using that alias |
| Upstream comparison, Rust-first implementation, IME | Earlier worklog records pinned upstream behavior and Phase A RED→GREEN; actual-App public-address test suppresses candidate-confirmation submit |

Added one missing focused UI check: a failed public address preview disables
public submission, but switching to private enables creation and sends exactly one
request with `aliasLocalpart: null`. It passed without a production change
(`/tmp/koushi-private-address-e2e.log`). This is browser-harness evidence, not a
new live-server run. The audit does not close the issue or substitute for the
remaining integrated gates, submitted-head CI, approval and merge.

## #858 / #856 / #850 source-to-acceptance audit

Re-read all three live Issue bodies and their checked-in regression tests. No
successful test was rerun for this documentation-only audit.

- **#858:** `DiagnosticDialog.test.tsx` resolves the accessible close name in
  English/Japanese, rejects the raw placeholder and checks dismissal. This covers
  the reported call-site defect. The Issue's suggested class-wide static
  placeholder-argument checker and optional development warning have **not** been
  implemented; do not claim class-wide prevention from this component test. Native
  VoiceOver was not rerun on this Linux host.
- **#856:** `mentionQuery.test.ts` covers start/space/newline, Japanese text and
  punctuation, opening brackets, repeated triggers, emoji UTF-16 offsets,
  semantic-atom boundaries and completed email text. `composer.mid-sentence.test.tsx`
  checks main/thread cursor insertion, candidate display, both surrounding text
  fragments, the resulting semantic mention and caret, and IME Enter suppression.
  Existing RED→GREEN and full frontend results supply execution evidence.
- **#850:** `thread-opening.spec.ts` admits Opening from a single actual-App pill
  click, checks visible loading and a disabled composer, then supplies Open or
  Closed and asserts the resulting UI with exactly one command. This covers the
  specified admitted-generation race without a timing-dependent server fixture.
  It is Chromium harness evidence, not a replay of the reporter's native test.

These mappings are not Issue closure, final-SHA CI or a goal-completion audit.
The #839 design approval and native #855 evidence remain outstanding.

## #839 Phase A: resolved demand state foundation

Added `koushi-state/src/avatar_demand.rs`: serializable account/session-qualified
scope demand with atomic monotonic observations, explicit closure, 256 visible /
eight prefetch bounds, visible-first resource deduplication and private-data-free
Debug. Missing avatar rows remain placeholders and produce no resource request.
Core's existing ViewBudget now imports the same 64-scope constant rather than
maintaining a second numeric definition.

The new contract test initially failed to compile because the API did not exist;
this is a missing-contract RED, not a reproduced runtime scheduler regression.
After implementation all four state tests passed. All 16 existing Core
view-scope-lifecycle tests also passed, as did Rust test structure, leaf boundary
and diff whitespace checks. Logs: `/tmp/koushi-avatar-demand-contract-red.log`,
`/tmp/koushi-avatar-demand-contract-green.log`,
`/tmp/koushi-avatar-scope-budget-check.log`.

This is a foundation only: the AppActor watch handoff, AccountActor reconciliation,
source-resolution/retirement integration, GUI migration and 1,500-target actual
network evidence are still pending. The ledger is not yet the live demand owner;
none of the new state tests proves runtime cancellation or scheduling by itself.

## #839 Phase A: AccountActor watch reconciliation

Added the latest-wins demand input to AccountActor and its internal handle.
The existing session-generation counter is now shared atomically with the
publisher; it is not a second generation counter. Incoming demand must match both
the actor's active account and generation. Pending watch updates are consumed
before queued messages/completions, and scope removal does not require a mailbox
slot. The existing downloader/in-flight map serves scoped resources without
manufacturing command request IDs. Command waiters remain supported during the
migration; their API is still scheduled for retirement with the GUI callers.

Reconciliation preserves active/shared resources, cancels unneeded work, rebuilds
scoped queued resources in visible-first order, and keeps excess demand in the
bounded state rather than caching Capacity as a terminal failure. Completion
reconsiders that deferred demand. No second downloader or persistent URI demand
registry was added.

A MatrixMockServer test publishes 20 observed resources, sees exactly six actual
media requests start, clears demand, and verifies no queued request or retry after
the delayed response interval. The initial test failed because the new handle API
was absent. After implementation, all six profile actor tests and five local-data
cleanup tests passed; formatting, test structure and leaf boundary checks passed.
Logs: `/tmp/koushi-avatar-watch-red.log`, `/tmp/koushi-avatar-watch-green.log`,
`/tmp/koushi-avatar-watch-regression.log`.

AppActor/source-resolution publication and connection/scope lifecycle integration
are still pending; currently only the new test publishes scoped demand. This is
not yet the live renderer migration, nor the required 1,500-target server proof.

## #839 deferred-capacity and terminal-failure cache evidence

Added a 264-resource actor test, exceeding the six-active plus 256-queued limit.
All 264 settle with the existing two-attempt budget (528 actual mock-server media
requests). Replacing the scope with the same resources settles entirely from the
terminal-failure cache without additional requests. All seven profile actor tests,
formatting, test structure and diff checks passed.

The initial fixture incorrectly assumed opaque non-image bytes would be rejected
by the renderable-thumbnail store; that store accepts opaque bytes as Ready. The
corrected fixture uses explicit HTTP 403 responses. This is a fixture correction,
not a production image-decoding defect, and the resulting test proves terminal
**failure** cache reuse, not Ready-resource lease/eviction handling. Ready cache
and live scoped resource retention still need integration evidence.

Removed watch-first biased selection so continuously arriving observations cannot
systematically starve commands/completions. The existing pre-message freshness
check still consumes pending demand before processing a completion.
Logs: `/tmp/koushi-avatar-capacity.log` (fixture failure),
`/tmp/koushi-avatar-capacity-corrected.log`,
`/tmp/koushi-avatar-capacity-regression.log`.

## #839 scope retirement to AppActor publication

Connected retirement to the existing runtime work notification. Dropping an owned
scope, retiring its consumer or ending its session now wakes AppActor even when
no reader job is queued. AppActor's existing work turn obtains the bounded
resolved-demand Arc from the shared scope registry, prunes entries against actual
scope liveness, validates its current account/session context and publishes the
result to AccountActor. An unchanged Arc is not republished; unrelated reader
notifications therefore do not create a cached-thumbnail feedback loop.

The new lifecycle test first failed at the missing wake timeout, then passed for
all three retirement paths after the change. It also checks that an unchanged
live snapshot preserves Arc identity and that missing session context discards
stored demand. All 17 lifecycle and seven profile actor tests passed, along with
test-structure and whitespace checks. Logs: `/tmp/koushi-avatar-retirement-red.log`,
`/tmp/koushi-avatar-retirement-green.log`,
`/tmp/koushi-avatar-retirement-regression.log`.

Remaining: authorized observation/source resolution must populate that registry
state (currently only its test does), including retained-data budget accounting;
portable command/model wiring, GUI migration and real-server scale evidence are
not yet complete. No renderer demand registry has been removed prematurely.

## #839 Ready cache eviction regression

A real-image scoped actor test reproduced an invalid Ready reference after filling
the existing 256-entry renderable LRU: the actor's metadata cache outlived the
image bytes. RED failed at `Ready must refer to retained bytes`.

Added a byte-copy-free availability check at the shared avatar cache-hit helper.
Both ordinary and scoped cache hits now discard unavailable Ready metadata and
reuse the existing SDK media path to restore bytes. The test turns GREEN with
exactly one server download across initial load and rehydration: the SDK cache
supplies the second load. Terminal failure caching is unchanged. Revalidation
occurs on observed demand, not every completion, to avoid an automatic refill
loop when visible demand exceeds the renderable LRU.

All eight profile actor and 14 renderable-thumbnail tests passed, plus test
structure and whitespace checks. Logs: `/tmp/koushi-avatar-ready-eviction-red.log`,
`/tmp/koushi-avatar-ready-eviction-green.log`,
`/tmp/koushi-avatar-ready-regression.log`.
This does not replace the remaining scoped resource-budget/lifetime integration,
source resolver, GUI migration or 1,500-target local-server evidence.

## #839 stable avatar identities and projected-state lookup

Added the portable `AvatarTarget` identity variants for own profile, room-scoped
users, room icons, Space icons and invite icons. The pure Rust resolver borrows
existing projected avatar data and performs no SDK call or membership probe. A
known room profile with no avatar suppresses the global fallback; an absent room
profile may use the existing global profile. Debug prints only target kinds.

New lookup tests and the four demand-state tests passed (six total), after the
lookup test initially failed for the absent API. Test-structure, platform boundary
and whitespace checks passed. Logs: `/tmp/koushi-avatar-target-red.log`,
`/tmp/koushi-avatar-target-green.log`.

This helper does not authorize observations or claim that every source's raw
payload is already projected. Core still needs revision/source/ownership admission,
raw timeline/member/reader resolution where required, retained-data charging,
protocol/adapter wiring and all GUI migrations before the typed identity path is
live. The required 1,500-target local-server acceptance remains outstanding.

## #839 installed reader identity admission

Reused installed reader metadata as the Core-only source for stable user-ID
lookup. The admission helper checks consumer ownership, live scope and the
acknowledged installed revision; lookup rejects users outside that installed
model and obtains an MXC only from existing Rust resource metadata. No MXC is
returned to a renderer API. The existing resource-byte API now shares this
admission path rather than duplicating its guards. Retaining the metadata Arc
retains its existing budget charge and does not copy image bytes.

Extended the installed-scope resource test with pre-ack, unknown-user,
wrong-owner, wrong-revision and retired-scope checks. It initially failed for the
absent API, then passed. All 17 lifecycle and 14 renderable-thumbnail tests also
passed, plus test-structure/whitespace checks. Logs:
`/tmp/koushi-avatar-source-red.log`, `/tmp/koushi-avatar-source-green.log`,
`/tmp/koushi-avatar-source-regression.log`.

This is installed-model admission, not complete live SDK/source qualification.
The observation handler must still acquire the live source/generation guard,
apply visible/prefetch bounds and feed budgeted resolved demand into the registry.
The renderer migration and real-server scale evidence remain incomplete.

## #839 live reader source guard

Shared the existing synchronous receipt-source commit guard between raw and
resolved windows instead of introducing another generation registry. Added the
reader-only observation callback: it retains the accepted charged raw source,
acquires the existing timeline-generation and receipt-epoch guards, then rechecks
the consumer's installed model before invoking the callback. Lock ordering stays
source-before-registry. Callbacks must be prepared synchronous operations.

Tests now cover invalid receipt epochs and replaced actor authority, and verify
that an installed model can remain readable while its retired live source refuses
new observation work. The raw callback API initially failed to compile because it
was absent; the new integrated test passed after implementation. All six receipt
endpoint and 18 scope-lifecycle tests passed, plus test structure/whitespace.
Logs: `/tmp/koushi-avatar-live-source-red.log`,
`/tmp/koushi-avatar-live-source-green.log`,
`/tmp/koushi-avatar-live-source-regression.log`.

This supplies reader-specific live-source admission, not the complete observation
command or other surface families. Budgeted demand commit, protocol/adapter/GUI
wiring and 1,500-target server evidence remain pending.

## #839 non-current demand input evidence

Added a focused actor test for input-side account/generation mismatch, distinct
from existing late-completion tests. Each case waits for an actual current-account
media request to start, then publishes an empty demand snapshot with a different
account or session generation. Both valid requests still settle; exactly two
server requests occur across the cases. Thus a mismatched empty snapshot cannot
cancel the current account's fetch. The existing implementation passed without a
production change. Log: `/tmp/koushi-avatar-stale-input.log`; test-structure and
whitespace checks also passed. The payload is synthetic transport data, not a
native image-rendering check. Observation command/GUI integration remains pending.

## #839 immutable scope payload retention

Before adding retained-data reservations to observation commits, removed deep
copies of every scope's resolved strings when a demand snapshot is cloned. Scope
payloads now use immutable Arcs; replacement creates a new payload only for that
scope and closing one snapshot does not mutate an older snapshot. Enabled the
existing serde dependency's `rc` feature so the serialized shape is unchanged;
no new package dependency was added.

The sharing assertion first failed on different string allocation pointers, then
passed along with serialization and independent-close checks. Six state tests,
18 lifecycle tests and nine profile actor tests passed. After removing the now
unneeded private payload Clone derive, the focused sharing/serialization check
passed again. Logs: `/tmp/koushi-avatar-retention-red.log`,
`/tmp/koushi-avatar-retention-green.log`,
`/tmp/koushi-avatar-retention-regression.log`,
`/tmp/koushi-avatar-retention-final.log`.

This is a prerequisite, not completed byte-budget accounting. Retained demand
reservations still need to travel with their payload owners through publication;
they must not be freed merely when the current scope map removes an entry while
AccountActor retains an older snapshot. No memory benchmark or completed
observation/GUI migration is claimed.

## #839 charged demand publication and reader observation commit

Added a Core-owned publication wrapper that carries per-scope ViewBudget
reservations alongside the immutable payloads. Snapshot clones share both payload
and reservation; removing a current scope does not uncharge a copy still held by
AccountActor. The watch and registry now carry this wrapper. Production publishing
requires an already charged value; the former uncharged fixture helper exists
only under `cfg(test)` and constructs a charged value before using the same path.
Accounting covers retained vector/string capacity and logical map/context data;
it is not a process-heap measurement.

Added the reader-specific observation commit: visible/prefetch limits, stable-ID
resolution from installed rows, live-source guard, final owner/consumer/revision
checks, and budgeted atomic scope replacement. A successful commit installs the
publication and wakes the existing AppActor work path. A failed reservation or
invalid observation leaves the previous demand/revision intact.

New budget tests prove delayed release and atomic capacity rejection. Reader tests
cover successful commit, duplicate sequence, foreign identity, oversized input
and retired source. Missing APIs first produced compile REDs; after implementation,
20 lifecycle tests, nine profile actor tests and six state tests passed. Normal
`cargo check -p koushi-core` passed; it still reports unused observation entrypoints
because public command/GUI wiring is unfinished. The focused reader check passed
again after the final consumer-liveness check. Formatting, structure and whitespace
checks passed.

Logs: `/tmp/koushi-avatar-budget-{red,green}.log`,
`/tmp/koushi-avatar-observation-{red,green,regression,check,final}.log`.
Remaining: portable CoreConnection/adapter observation entrypoint, remaining source
families, GUI migration/removal of old URI APIs, diagnostics and 1,500-target QA.

## #839 non-placeholder reader observation evidence

Strengthened the reader observation fixture from an avatar-less row to an actual
NotRequested avatar identity. It now verifies stable user-ID resolution into the
charged demand snapshot while serialized reader rows contain no MXC URI. The
existing invalid-sequence/identity/capacity/retired-source assertions also pass
with this nonempty demand. No production fix was necessary. Focused test,
structure and whitespace passed; logs:
`/tmp/koushi-avatar-reader-identity.log` and
`/tmp/koushi-avatar-reader-identity-final.log`.
Public command/GUI wiring remains pending; this is not an end-to-end download test.

## #839 public reader observation API and runtime release

Connected `ReaderSubscription::observe_avatars` to the existing observation
commit. The host supplies only installed revision, monotonic sequence and visible/
prefetch user IDs. AppActor initializes an empty current-context publication before
reader work admission; observations read that existing context rather than adding
another command queue, counter or context registry. If the context is cleared or
changes during resolution, commit rejects the captured context instead of
recreating it. The subscription close signal rejects further observations.

Tests cover the context-derived commit, inactive/closed public subscription, and
cleared/replaced context rejection. The new API initially produced compile RED.
Also reproduced a real shutdown retention failure: surviving connection registry
handles kept the current charged publication after runtime shutdown. Shutdown now
clears that publication; external snapshot owners still retain their own charges.
The same shutdown test turned GREEN.

21 lifecycle, 22 connection and nine profile actor tests passed; normal Core check,
format, test-structure and whitespace passed. Logs:
`/tmp/koushi-avatar-current-context-{red,green}.log`,
`/tmp/koushi-avatar-shutdown-budget-red.log`,
`/tmp/koushi-avatar-reader-api-{regression,check}.log`.

The portable reader API is now reachable; desktop adapter/TypeScript/GUI wiring,
other source families, full request-count QA and final gates remain unfinished.

## #839 reader controls independent of pending receive

Adapter inspection found that `receive_receipt_reader` holds the subscription
mutex during its pending receive. Sending observations through that mutex would
wait for the very delivery the observation must trigger; existing window/ACK/
resource operations also used that mutex.

Added `ReaderSubscriptionControl`, a capability over the same consumer and scope
ID, not another owner/registry. It does not retain OwnedViewScope and shares the
existing close signal. Subscription convenience methods delegate to it. Desktop
entries retain the control capability; ACK/window/resource commands now use it
without acquiring the receiving subscription mutex. Removed the obsolete internal
OwnedViewScope window-update forwarding method.

The new API initially produced compile RED (not a reproduced native deadlock).
A headless test holds a real delivery future pending while observation, ACK,
window-update and resource operations return through the control path; close
settles the pending receive and rejects further observations. 23 connection and
21 lifecycle tests passed. Desktop library cargo check, formatting, structure and
whitespace passed. Logs: `/tmp/koushi-reader-control-{red,green,regression}.log`,
`/tmp/koushi-reader-control-tauri-check.log`.

The desktop observation command/DTO and frontend invocation still remain to be
added; other avatar source families and scale/GUI evidence are not completed.

## #839 desktop observation wire

Added `ReaderAvatarObservation` to the portable protocol and mirrored it in
TypeScript. Installed revision and sequence retain decimal-string u64 encoding;
Debug exposes counts, not user IDs. Registered the Tauri observation command and
DesktopApi/client method. The command resolves its owned subscription entry and
uses the non-receiving control capability, without taking the delivery mutex.
The payload contains stable user IDs only, never host-supplied MXC identities.
Classified the method with existing scoped observations in the ordered transport
contract; added it to the test IPC void-command inventory.

The missing DTO produced compile RED and the missing client method produced a
focused test failure. After implementation, four protocol view tests and all 29
client tests passed, including maximal u64 round-trip, numeric-sequence rejection,
private Debug and exact large-string client forwarding. Typecheck, frontend lint,
Tauri library check, test structure, adapter/snapshot/protocol boundary and
whitespace checks passed. Logs: `/tmp/koushi-avatar-wire-red.log`,
`/tmp/koushi-avatar-client-red.log`, `/tmp/koushi-avatar-wire-green-final.log`,
`/tmp/koushi-avatar-client-green.log`, `/tmp/koushi-avatar-wire-tauri-check.log`,
`/tmp/koushi-avatar-wire-lint.log`.

Receipt popup geometry reporting is not yet connected to this API. Other source
families, Rust-driven source invalidation/re-resolution, removal of old renderer
URI demand, 1,500-target QA, final gates and PR/merge remain pending.

## #839 full reader popup geometry reporting

Connected the opened reader popup to typed observations after the displayed model
has been acknowledged. Reports contain intersecting row IDs and at most eight
geometrically nearby candidates, not resource identities. Scroll/window resize and
popup/row ResizeObserver notifications coalesce through one animation frame;
unchanged geometry does not re-report. Sequence uses bigint/string, and cleanup
cancels pending measurement and removes listeners. Full reader rows no longer use
the legacy URI request callback; compact summaries still do and remain to migrate.

Separated the no-source compact fallback from subscription lifetime. A compact
summary/count update no longer tears down a live reader with the same source.
The missing geometry report first failed its component assertion. With original
subscription dependencies restored as an isolated baseline, the same test also
failed on a second subscription; restoring source/open-only lifetime turned green.

All six reader component tests, typecheck and frontend lint passed. Coverage
includes pre-ACK suppression, visible/prefetch IDs, scroll and resize, stable
subscription, close and cancellation of a queued measurement. Geometry is mocked
in jsdom; this is not browser/native layout or actual-download evidence.
Logs: `/tmp/koushi-reader-geometry-red.log`,
`/tmp/koushi-reader-summary-churn-red.log`,
`/tmp/koushi-reader-geometry-verified.log`.

Remaining work still includes other surfaces, Rust-driven source re-resolution,
old global URI demand removal, browser/live-server scale evidence and final gates.

## #839 browser popup geometry evidence

Added an actual-App Chromium check with 80 full reader rows and a bounded
three-reader compact summary. It compares observed IDs with real popup clipping,
checks eight nearby candidates, scroll-driven changes and no observations after
close. The fixture explicitly supplies a committed projection request; the first
attempt used the generic helper's null request and therefore exercised only the
compact fallback, not the full reader.

The corrected fixture exposed a separate test-infrastructure defect:
`TauriIpcMock` classified `receive_receipt_reader` as void and discarded model
replies. A focused unit test reproduced that loss, then turned green after
preserving delivery/null values and making the unconfigured default null. This
was a mock correction, not a product/backend deadlock claim.

All 1,303 frontend tests in 112 files passed. All 15 tests in the timeline message
state browser file passed with one worker. Typecheck, frontend lint and whitespace
passed. Logs: `/tmp/koushi-reader-mock-red.log`,
`/tmp/koushi-reader-mock-frontend-full.log`,
`/tmp/koushi-reader-geometry-browser-green.log`,
`/tmp/koushi-reader-browser-file.log`, `/tmp/koushi-reader-browser-static.log`.

This proves real browser geometry and command emission against the harness, not
native WebView behavior or server media request counts. Remaining source families,
source re-resolution, old URI demand removal, live-server scale and final delivery
requirements remain open.

## #839 current binding versus installed-model authority

Found and reproduced an authority mix-up: after Rust published a new private
avatar URI, an observation against the still-installed older model restored its
old URI. Installed metadata now authorizes user IDs only. Resource resolution
uses the existing mailbox's latest projection, then in-flight projection, then
installed metadata; no additional URI cache or registry was added. An identity
that no longer has a current binding resolves to a placeholder, not its old URI.

The behavioral test failed with the old URI where the new URI was expected. It
now covers pending and in-flight replacement plus a newer removal superseding an
in-flight binding. All 21 lifecycle and 23 connection tests passed, together with
normal Core check, format/structure/whitespace. Logs:
`/tmp/koushi-reader-current-uri-{red,green,regression,check}.log`.

This closes stale re-observation restoring an old resource. It does not yet make
source changes refresh existing demand without another observation: retaining
charged stable observation IDs and using the existing reader publication path
for that refresh is still required. Other surfaces and final delivery remain open.

## #839 reader reprojection refresh and cache-echo convergence

ReaderRequest now retains its accepted stable user IDs and session context, with
backing data charged to the existing ViewBudget. Successful reader publication
resolves those IDs against the new private metadata and atomically refreshes the
scope's resource demand alongside the model commit. URI changes and removals no
longer wait for host re-observation. Refresh preserves the host sequence, skips
unchanged resources and does not revive an absent/different-context demand.
Model serialization remains outside source/registry locks; the bounded resource
refresh performs no I/O. No additional registry or downloader was introduced.

The no-re-observation test first failed with the obsolete URI, then passed for
replacement/removal. Tests also cover retained-ID budget release, failed refresh
preserving prior demand, refresh not consuming a host sequence, and unchanged
bindings preserving publication identity.

A second behavioral RED exposed cached-result echoes on revision-only host
observations, which can invalidate reader models again. AccountActor now separates
Ready-byte revalidation from cached-action replay. It still revalidates bytes on
observations, but does not replay cached actions when scope resource interests
are unchanged. A real mock media request followed by a deterministic actor barrier
proves no second cached update; eviction/rehydration and failed-cache tests still
pass.

10 profile, 22 lifecycle, 23 connection and six state tests passed (61 total), plus
normal Core check, formatting, structure and whitespace. Logs:
`/tmp/koushi-reader-refresh-{red,green,regression,final-check}.log`,
`/tmp/koushi-reader-cache-echo-red.log`.

This completes automatic re-resolution for the existing full-reader source path,
not other avatar surfaces or the required both-server 1,500-target scenario.
Global renderer URI demand removal, broader final gates and single PR/merge are
still pending.

## #839 refresh boundary coverage

Added direct state coverage for the new refresh path: visible/prefetch overflow
leaves prior demand intact; changed bindings preserve older snapshots; refresh
does not consume host sequence numbers; closed scopes cannot be revived; and a
new scope remains a new cache consumer even when it shares an existing URI.
`cargo test -p koushi-state --test avatar_demand` passed all five tests, logged in
`/tmp/koushi-avatar-refresh-boundaries.log`. This is additional coverage of the
existing implementation, not a newly reproduced product defect or new RED claim.
Other surfaces, actual large-server scenarios and final delivery remain open.

## #839 actual media-request measurement primitive

Extended the existing QaTcpProxy with a count-only media-read counter. It counts
successfully written upstream HTTP download/thumbnail requests, including retries,
not render rows, cache hits or successful response statuses. Authenticated client
v1 and legacy media v1/r0/v3 paths are recognized; upload, sync, config, malformed
resource paths and non-GET requests are excluded. No URLs or credentials are
retained in the new counter.

A bounded real-TCP test forwards requests through the proxy to a local HTTP
fixture and checks counts after each response. The initial RED was missing-API
compile failure, not a reproduced product failure. All 104 headless-core-qa unit
tests pass; structure/whitespace checks pass. Logs:
`/tmp/koushi-qa-media-counter-{red,green,regression}.log`.

This is the measurement primitive only: the counter is currently exercised by
its test. The 1,500-member disposable Tuwunel/Synapse scenario still needs to
consume it and has not run. No new success token or server acceptance claim is
made here. Remaining surface migration and final delivery are also open.

## #839 media counter connected to disposable-server QA

The existing media stage now routes its runtime sessions through QaTcpProxy and
requires a positive observed upstream media-read delta before `recv_media=ok`.
The proxy lives through session restarts and uses the existing configured server
identity. It emits only `media_http_requests=<count>`; it does not expose resource
identities. This makes the counter production-used by QA, rather than test-only.

The 104 headless-core-qa unit tests passed. The normal documented command
`qa:headless-local -- --server=both --scenario=media --core --timeout-ms=240000`
passed on disposable Tuwunel and Synapse. Both recorded `media_http_requests=1`
and completed the existing media/caption/edit/restore-cleanup checks. Evidence:
`/tmp/koushi-qa-media-live-tests.log` and
`/tmp/koushi-qa-media-live-both.log`.

This establishes that SDK media reads traverse the measured path on both real
backends. It is not the 1,500-member/avatar demand scenario, and proves neither
avatar cancellation nor viewport request bounds by itself. Those requirements
and the remaining surface migration/final delivery remain open.

## #839 1,500-reader server fixture builder

Added `scripts/lib/avatar-demand-fixture.mjs` using the existing Node registration,
room, join, message and read-marker helpers. It creates 1,500 synthetic readers,
uploads a distinct PNG resource for each, sets profiles before joining, then sends
one target message after membership events and adds all receipts. Eight requests
at most run concurrently; failed batches settle before rejecting with a closed,
credential-free error. Duplicate media URIs fail rather than weakening the test
to one shared resource. Returned metadata contains only room/event IDs and count.

This keeps fixture seeding outside the Core-only command/event QA flow. The
builder is not yet invoked by the runner: Core scenario integration remains the
next step, including its source/metadata admission and observation checks.

Three fixture tests passed using mocked HTTP responses: full population/order and
concurrency, duplicate-resource rejection, and bounded/sanitized batch failure.
Typecheck and lint passed after adding the ordinary `.d.mts` declaration used by
other script modules. Evidence: `/tmp/koushi-avatar-fixture-final.log`. These are
fixture tests, not 1,500-reader real-server or avatar-demand acceptance evidence.
No product RED or backend-scale completion claim is made.

## #839 full fixture-generation check and Synapse setup correction

The bounded verification driver `/tmp/koushi-check-avatar-fixture.mjs` uses the
existing server start/stop helpers and the committed fixture builder, writes only
room/event/count metadata into its private run directory, and stops its owned
server. Tuwunel created all 1,500 readers successfully. Synapse initially failed:
16 joins succeeded and eight returned HTTP 429. The pinned Synapse 1.157.0 source
confirms `rc_joins_per_room` separately defaults to one join/second, burst ten;
existing local QA `rc_joins.local/remote` settings do not override it.

A failing config test precedes adding the separate local-only setting at the same
1000/1000 limits as other QA rate limits. The corrected attempt produced 1,208
registrations (plus owner) without 429 before its 110-second investigation limit.
It timed out and is not success evidence; process/container inspection found no
remaining server owned by that attempt. Unrelated containers were left untouched.

The complete 1,500-reader fixture-generation check is now budgeted as a full gate
at 240 seconds per server (230-second process deadline plus cleanup margin):
`timeout --kill-after=8s 230s node /tmp/koushi-check-avatar-fixture.mjs synapse`.
This longer attempt is based on measured progress, not an unbounded retry. It
completed successfully: Synapse created all 1,500 readers, distinct media resources
and target receipts. Together with the earlier Tuwunel success, both fixture
builders now have real-server evidence. The 15 local-homeserver helper tests also
pass. Logs: `/tmp/koushi-avatar-fixture-tuwunel.log`,
`/tmp/koushi-avatar-fixture-synapse-full.log`, and
`/tmp/koushi-avatar-fixture-join-limit-{red,green}.log`.
This remains fixture setup evidence only, not Core avatar-demand request bounds,
cancellation, cache/reopen or final acceptance.

## #839 fixture image validity correction

Inspection found the initial embedded PNG had an invalid IDAT CRC despite being
accepted by both upload endpoints. A test checking every PNG chunk CRC and
inflating its pixel data failed at IDAT before the payload was corrected. The
same three fixture tests now pass, together with typecheck/lint. This is a real
fixture-data RED→GREEN, not a product rendering regression. Earlier upload-only
results never established image validity or native rendering.

Reran the full 1,500-reader fixture generation with corrected bytes on both
disposable servers; Tuwunel and Synapse succeeded. Evidence:
`/tmp/koushi-avatar-fixture-png-{red,green,tuwunel,synapse}.log`.
Core observation/request bounds, cancellation/shared/cache checks, remaining GUI
surfaces and final PR/merge remain unverified or unfinished.

## #839 live Core reader-scope admission path

The existing live-signals stage now exercises public Core reader subscription
against real SDK receipt data before emitting its existing `read_receipt=ok`
marker. It obtains an observed timeline source, receives and ACKs a matching
reader model, observes the known reader identity, closes the subscription and
requires a late observation to fail with Closed. `reader_scope_live=ok` records
this extra check; failures fail the existing stage.

The first attempt timed out because it expected a projection request ID on an
already-committed replay. Source inspection confirmed that committed replay may
omit that identity. The QA check now deliberately unsubscribes/resubscribes its
owned timeline to obtain a fresh observed identity rather than guessing one.
This was a QA setup correction, not a product fix. An initial module-path compile/
format error was corrected separately and is not product RED evidence.

`qa:headless-local -- --server=both --scenario=live_signals --core
--timeout-ms=240000` passed on both Tuwunel and Synapse, including the new marker
and existing read/typing/presence/cleanup checks. All 104 headless-core-qa unit
tests and Rust test-structure/whitespace checks passed. Evidence:
`/tmp/koushi-reader-live-both-corrected.log` and
`/tmp/koushi-reader-live-final-tests.log`.

This covers live source/ACK/admission/close, including placeholder identities;
it does not assert avatar HTTP counts. Connecting the 1,500-reader fixture to
this observation path, media bounds/shared/cache/cancellation checks, remaining
GUI migration and final delivery are still required.

## #839 live observation → download → scoped bytes and cache reopen

The Core live-signals fixture now assigns its reader B a real PNG before login,
using the same upload/profile setter as the 1,500-reader builder. Only Core
`live_signals`/`all` fixtures opt into this setup; SDK-only and other scenarios
retain their setup. Live-signals sessions now use the existing measured proxy,
without accidentally running the media stage for a live-signals-only scenario.

The public reader check observes B, waits for Ready, and reads the PNG through the
ACK-qualified scoped resource API. It closes/reopens the source, accepts Ready
from either the first model or a subsequent model, and validates scoped bytes
again. Demand remains open for a 250 ms bounded no-additional-HTTP observation
interval per phase. This interval is not an AccountActor barrier or an unlimited
future guarantee. Both closed scopes reject late observations.

On Tuwunel and Synapse the initial phase recorded one HTTP media read and the
reopen phase still recorded one total read (zero extra). Both complete
live-signals lanes passed. An intermediate reopen timeout was a QA waiting error:
it waited for another Ready delivery despite a Ready first model, not a product
cache failure. No new product RED is claimed.

32 focused frontend/script tests, 104 headless-core-qa unit tests, typecheck,
lint and Rust test-structure checks passed. Evidence:
`/tmp/koushi-reader-cache-both-corrected.log` and
`/tmp/koushi-reader-download-final-checks.log`.

This closes the one-reader real download/scoped-byte/cache-reopen evidence gap.
The 1,500-reader fixture is not yet connected to Core observations; its request
bounds, live in-flight cancellation/shared/account-retirement cases, remaining
surfaces and final delivery remain open.

## #839 live shared-scope resource retention

Extended the real reader-avatar check to keep its first subscription open while
a second scope observes the same reader. Once the second scope has Ready bytes,
the first is closed and dropped; the surviving scope must still read its scoped
PNG and accept a renewed observation. Both scopes are then closed before the
existing reopen phase. Late observations on closed scopes remain rejected.

Both Tuwunel and Synapse passed the full live-signals lane. Initial, shared and
reopen phases each reported one total media HTTP request (no additional read in
the latter phases), including the existing 250 ms open-demand observation
intervals. The 104 QA unit tests and Rust test-structure/whitespace checks passed.
Evidence: `/tmp/koushi-reader-shared-{tests,both}.log`.

This proves shared Ready-resource access across closing/dropping one owner and
cache reopening on real backends. It does not prove sharing during an unfinished
download, in-flight cancellation, account retirement, or the 1,500-reader Core
scenario. Those checks, remaining surface migration and final delivery remain
open; no overall completion claim is made.

## #839 dedicated 1,500-reader Core QA: Synapse green, Tuwunel blocked

Added the dedicated `avatar_demand` scenario to the Node runner and Core registry,
with required `--core` and mandatory `avatar_window_requests=ok`. It is separate
from `all`, like other dedicated scenarios. The Node fixture returns only
room/event/count metadata through `KOUSHI_QA_AVATAR_FIXTURE`; Core authentication
and product operations still use CoreCommand. The SDK smoke leg is skipped for
this Core-only scenario.

The Core flow explicitly prepares fixture member metadata through LoadRoomSettings,
then obtains a fresh timeline source and a maximum-32-row reader window. It waits
for at least 1,500 readers within a fixed deadline, requires 16 image-bearing
identities, observes eight visible plus eight prefetch, and reads all 16 Ready
PNGs through ACK-qualified scoped resources. No media request may precede the
observation. Initial and reopen phases require exactly 16 total HTTP media reads,
including 250 ms bounded no-extra-request intervals. Metadata preparation is
separate from viewport demand; this is not proof of lazy People-profile loading.

Synapse passed the full dedicated scenario:
`qa:headless-local -- --server=synapse --scenario=avatar_demand --core --timeout-ms=240000`.
Both phases recorded 16 HTTP reads, with no extra read on reopen. Evidence:
`/tmp/koushi-avatar-scale-synapse.log`.

Tuwunel 1.7.1 remains a failing acceptance case. After correcting the known QA
fresh-source acquisition and waiting for population updates, its Core model
remained at total_count=1 until timeout. Direct server-only diagnostics using the
same Simplified Sliding Sync endpoint and fixture confirmed an HTTP-200 initial
response with one receipt user on Tuwunel, versus 1,500 on Synapse. No alternative
sync backend was used. This localizes the missing population upstream of Core;
response construction is now identified as lossy: v1.7.1
[`collect_room`](https://github.com/matrix-construct/tuwunel/blob/v1.7.1/src/api/client/sync/v5/extensions/receipts.rs#L75-L93)
collects the individual receipt events and calls
[`pack_receipts`](https://github.com/matrix-construct/tuwunel/blob/v1.7.1/src/service/rooms/read_receipt/mod.rs#L276-L304),
whose `json.insert(event, receipt)` replaces the preceding nested receipt map for
the same event. It therefore retains only the last user's map instead of merging
receipt types and users. This matches the direct HTTP and Core observations.
Inspection of the upstream default branch also found this insertion in
`pack_receipts_fallible`; merely choosing an unverified newer build is not a fix.
No upstream files, shared server binaries or services were modified. An upstream
correction and the unchanged real-server gate turning green remain required.
Earlier fixture creation results proved accepted seed requests, not receipt
readback completeness. No threshold reduction, skip or fallback was added.
Evidence: `/tmp/koushi-avatar-scale-tuwunel-settle.log`,
`/tmp/koushi-avatar-scale-server-receipts.log`, and
`/tmp/koushi-avatar-scale-synapse-server-receipts.log`.

43 frontend/script tests, 104 Core QA unit tests, typecheck, lint and Rust test
structure passed (`/tmp/koushi-avatar-scale-final-checks.log`). The no-Core CLI
guard also rejected invocation before server startup. These passes do not turn
the failing Tuwunel scenario green. Todo #7 tracks that unresolved server-side
population boundary. Large-window scrolling/in-flight cancellation/account
retirement, remaining UI surfaces and final gates/PR/merge are still outstanding.

## #839 scrolling regression: range-cache fix green, live transition still failing

Extended `avatar_demand` to move the same reader scope from index 0 to 32 and
back, then close/reopen. Each phase observes eight visible plus eight prefetch
identities. The moved set must be disjoint, and the returned/reopened set must
match the original. Expected cumulative HTTP counts are 16/32/32/32. The model
wait requires the requested start, so a stale previous-window delivery cannot
satisfy the check. This extension initially failed; the follow-up below records
its resolution.

The first Synapse attempt passed initial HTTP16 but retired on scroll. A focused
Core regression then demonstrated that update_reader_window reused accepted raw
input from the old range: start=1 still yielded cached raw (RED). The correction
marks changed start/limit as source-dirty and preserves an already pending source
invalidation. Four cases cover moved start, changed limit, pending source change,
and legitimate reuse for an unchanged clean range. The regression and all 23
scope lifecycle tests passed. Evidence: `/tmp/koushi-avatar-scroll-{red,green}.log`.

After that correction, the live Synapse test no longer reported immediate
retirement but timed out before delivering a model at the new start
(observed_total=0). Servicing the ordinary Core event stream concurrently did not
resolve it; that experimental change was removed. The unresolved delivery issue
was tracked as Todo #8 and resolved in the follow-up below. Evidence: `/tmp/koushi-avatar-scroll-synapse{,-green,-drained}.log`.
No deadline, population or request-count requirement was weakened, and no
scroll/return/cache success is claimed for the expanded lane.

Fresh Core library gate: 1,031 passed, nine ignored (four Rust test workers).
Core QA unit gate: 104 passed. Format, Rust test-structure and whitespace passed.
Evidence: `/tmp/koushi-avatar-scroll-regression.log`. At that checkpoint, these
unit successes did not resolve the live transition or upstream Tuwunel blocker.

## #839 scoped completion notifications: full Synapse window flow green

Resolved Todo #8. Temporary count-only tracing showed that the moved model at
start=32 was successfully published, delivered and ACKed, and all 32 media HTTP
requests completed. The timeout was waiting for an image-completion reprojection,
not failure to deliver the initial moved model. Tracing was removed after use;
no IDs/URIs were printed by the temporary probes.

Two defects were independently reproduced in the existing runtime projection
regression:

- Completion dependencies used frozen raw SDK URI hints instead of current
  projection URIs. Current room-profile URI changes therefore failed to schedule
  reader work, while obsolete hints remained indexed (RED). The runtime now passes
  current resource identities into the existing charged ProfileRegistration;
  raw identity/owner data stays unchanged. No second registry was introduced.
- AvatarThumbnailUpdated notified scopes only when reduction emitted a global
  Profile/RoomList/LiveSignals UI effect. A scope-only resource need not be in any
  such projection. A no-global-effect action reproduced the lost notification
  (RED). Indexed scope notification now runs independently of global UI effects.

The same runtime test is green and asserts both current-URI invalidation and
absence of obsolete-URI invalidation. The complete Synapse 1,500-reader scenario
now passes: initial/scroll/return/reopen cumulative media HTTP counts are
16/32/32/32, with eight visible plus eight prefetch users per phase, disjoint moved
identities, original identities on return, scoped PNG reads and the unchanged
bounded quiet intervals. Evidence:
`/tmp/koushi-reader-thumbnail-index-{red,green}.log`,
`/tmp/koushi-reader-scope-effects-{red,green,synapse,regression}.log`.

Fresh Core lib: 1,031 passed, nine ignored; QA unit: 104 passed. Format, Rust
structure and whitespace checks passed. Temporary probe evidence is in private
child logs; their count-only extraction is
`/tmp/koushi-avatar-scroll-demand-summary.log` (parent run logs:
`/tmp/koushi-avatar-scroll-{trace,mailbox,demand}.log`).
The QA failure report now includes its actual cumulative media request count to
avoid confusing missing image updates with missing initial models.

Tuwunel remains blocked by its upstream receipt packing defect (Todo #7), with
cross-repository changes awaiting user confirmation. In-flight cancellation,
account retirement, remaining avatar surfaces and final PR/merge requirements
remain open. This checkpoint is not overall goal completion.

## Latest user decisions: #839 approved, native check deferred

The user explicitly approved the #839 avatar-demand design (「承認」). Updated
its status and the overview/state-machine canon before implementation. Earlier
approval-wait checkpoints above are historical, not current blockers.

The subsequent instruction 「macOSはあとでチェックするのでいい」 transfers #855
macOS WebView verification to a later user check and removes it from this batch's
implementation/PR/merge blockers. Native verification remains **not performed**;
Linux/Chromium, macOS cargo-check and release packaging are not substitutes.
The final handoff must disclose the deferral, not claim native acceptance evidence.
All other goal requirements, including #839 real request-count evidence, final
local/CI gates, human PR approval and merge/main verification, remain required.

## Remaining investigation and implementation

- #847: Core ignored-sender suppression is reversible per-item. The pinned SDK
  `event_cache/tasks.rs::ignore_user_list_update_task` calls `clear_all_rooms`,
  which clears persisted linked chunks as well as live caches. Its upstream
  timeline integration test explicitly expects Clear and then adds new events.
  Therefore the issue's retained-event/no-network repair suggestion is not yet
  proven safe: do not restore the indexed SDK accumulator from an unrelated
  snapshot or claim the cache still contains the old authoritative history.
  The actual SDK/Core relay regression now proves both ignore/unignore recover
  without new events/restart/viewport requests; owner-level one-page recovery is
  implemented under the approved canonical amendment above. Complete the local
  server lane and cancellation/failure boundary checks before issue closure.
- #839: audit all issue acceptance against #857 and its scoped-reader worklog;
  do not rewrite merged functionality or use broad CI as request-count evidence.
  Verify remaining surface ownership, full-reader access and actual bounded
  request/cancellation behavior on local servers.
- #838: inspect current room-create/room-info contracts and upstream equivalents,
  then implement Rust-owned suggestion/validation/submission and sharing identity
  before thin GUI wiring. Prove collisions, manual edits, Japanese/IME, private
  rooms, alias changes/no-alias URLs and localized copy feedback.
- Finish missing regression cases and required native evidence; run applicable
  repository gates, self-review coherent diff, open one PR, satisfy exact-head
  CI/approvals, merge and verify main. Keep the durable goal active until all
  requirements have concrete evidence; report actual blockers rather than
  declaring partial success complete.
