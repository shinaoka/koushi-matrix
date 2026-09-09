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
