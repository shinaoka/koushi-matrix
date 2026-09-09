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
