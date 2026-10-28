# Bounded scrollback experiment

Base: origin/main (70673a89). The bounded-window improvement is selected for release; the remaining native jump is tracked separately.

## Design

Replace leave-live-edge full-prefix materialization with a finite moving display
window. Ordinary viewport observations in the interior do not change membership.
Near either window edge, expose at most 60 additional canonical slots, preserving
the observed first/last event. Retain at most 240 canonical display slots (visible
span takes precedence if larger). React still owns geometry and its existing
single stabilization transaction. No timer or input-silence dependency is added.
Pending sends retain their existing separate admission bound.

Core normalizes physical-bottom observations against the represented canonical
end: a historical window's bottom is not the live edge. The renderer must also
use the authoritative live-latest event when deciding to follow new messages.
Cached-history movement precedes SDK backward pagination. Replay retains the
current finite display window instead of expanding the canonical prefix.
Room SDK batches enforce the display budget around current viewport identities;
missing/stale identities must not cause an arbitrary relocation. Focused/thread
behavior remains outside this experiment.

## Limits and verification

The budget bounds additional display slots, not total RSS: navigation_items and
SDK event-cache retention, media bytes, and per-item body sizes are existing
independent owners. No process-memory plateau claim is made by slot-count tests.
A complete production memory budget still needs byte accounting and profiling.
The experiment does not establish a fix for asynchronous native-scroll rollback.

Add a RED headless regression before implementation, then verify interior
stability, edge steps, bidirectional traversal, stale evidence, bounded repeated
movement, pending sends, and diff replay convergence. Run existing projection and
viewport checks. Native WKWebView inertia remains a separate confirmation.

Canon consulted: REPOSITORY_RULES.md; overview Timeline Viewport And Scrollback;
state-machine canonical/display projection; agents/verification.md.
Upstream research: Element Web docs/scrolling.md (layout absorption and relative
corrections); Element X iOS TimelineTableViewController (conditional snapshot
publication); Element Web shared TimelineView (stable keys, one geometry owner).
This is an original experiment, not copied upstream code.

## Review correction

The first design was rejected for missing edge semantics and publication fencing.
The revised experiment uses a Core-provided window revision and reaches-live-end
flag in navigation metadata. Window revision changes on SDK batches, recovery and explicit window
relocation; renderer observations echo that revision. Core rejects mismatches
before read-state or relocation. Observations without a revision are conservative:
no relocation, and no semantic at-bottom when the suffix is hidden. The renderer
reports only after the corresponding projected boundary is present and uses the
Core reaches-live-end flag in every physical-bottom follow path. Defaults preserve
old fixtures, while production room navigation always provides the metadata.
Each relocation keeps the observed event span and moves the consumed edge at least
60 slots away; duplicate evidence cannot advance a second time. Hidden-only spans
are scanned without retaining/cloning them until a renderable page is found;
page construction caps raw retained slots and excludes suppressed rows. SDK
appends outside a historical suffix remain hidden. Cached backward demand emits
correlated Idle with prepend_expected=false after publishing its page, and only
reaches SDK pagination once the cached prefix is exhausted. Replay uses current
bounded membership, with last viewport identity as a bounded fallback.

Design review: independent read-only review rejected the first draft. Revised
design accepted with mandatory boundary-identity validation, invalidation on SDK
boundary changes, and both movement directions using the existing pre-layout
projection stabilization transaction. These conditions are included in canon.

## Implemented experiment and evidence

- Room initialization, authoritative recovery, cached traversal and explicit
  ReturnToLive construct bounded display membership; SDK Reset selects a bounded
  candidate before materializing rows. Room replay preserves current membership.
- Revision/first-last identity metadata fences renderer observations. Physical
  historical bottom cannot follow live. Gap-only evidence may wake gap repair
  but cannot imply reading. Explicit ReturnToLive uses typed Core routing.
- Visibility changes rehydrate omitted canonical owners while retaining the
  newest represented reply per root and a hidden canonical-end cursor. Arbitrary
  visibility expansion can change which rows fit, including viewport endpoints
  when newly revealed rows or reply owners consume the finite budget.
- RED: initial small-scroll test published the entire hidden 10,000-item prefix.
  GREEN: same regression passes, as do bounded traversal, stale evidence, hidden
  runs, pending rows, thread-root visibility, Reset payload-visit and explicit
  live-return regressions. Core library: 1,809 passed / 11 ignored. Renderer:
  64 focused tests passed. Existing Chromium viewport transaction suite: 5 passed.
- Frontend typecheck/build, touched-source ESLint, command/snapshot, SDK gitlink,
  test-structure and agent-doc checks passed. Native debug app built with updater
  artifacts disabled; experimental builds require no updater private key.

Build from the worktree:

```sh
KACHE_EXPLAIN_MISS=1 npm --prefix apps/desktop run tauri -- build --debug --bundles app --config '{"productName":"Koushi Scroll Experiment","bundle":{"createUpdaterArtifacts":false}}'
```

Native WKWebView gesture continuity and total-memory plateau remain unverified.
The existing renderer stabilization engine is retained, so this experiment
isolates Core display-window churn and is not a claim that native rollback is
fully fixed. SDK/canonical retention, payload-byte budgets, media caches and
large mixed-batch peak memory remain outside this experiment's slot-count bound.
The normal build configuration and updater signing policy are unchanged.

Final independent review: approved for the experimental branch after the reply
budget regression was corrected. Final focused projection run: 54 passed;
Tauri navigation command tests: 7 passed. Main self-review traced the registered
IPC -> typed command -> manager -> actor -> window metadata/diffs -> renderer
boundary checks -> existing stabilization transaction path. No PR or release gate
claim is made; no installed application or existing user profile was changed.

## Follow-up investigation

Native feedback reports substantial improvement with a possible remaining jump.
A native diagnostic candidate ends a projection transaction with missing-anchor
while history crosses the virtualization threshold. A new synthetic two-page
history test (including a mixed Set update) preserves the original visible row
in both Chromium and WebKit. All six viewport transaction cases now run in both
engines: 12 passed. This does not reproduce or explain the remaining native jump;
threshold crossing alone is not sufficient. Further reproduction must include
native input continuity and overlapping projection/measurement transitions.
Test CSS is loaded through Vite so imports resolve consistently in both engines.
No production code or installed application changed during this investigation.

## Merge preparation

The user selected the verified bounded-window improvement for merge and v0.31.0
after bounded further investigation did not reproduce the residual native jump.
Follow-up: [#1653](https://github.com/shinaoka/koushi-matrix/issues/1653).
The help page now explains older-history scrolling and return to latest without
exposing the implementation's slot budget. The proposed full list-engine
replacement remains separate. Production review additionally requires sparse
SDK Reset selection, ownership-preserving visibility rehydration, and current
window revisions in headless QA observations; these corrections have RED cases.

Production review also found that authoritative recovery must keep a historical
window when its owners survive. Recovery now carries current viewport facts,
seeds the prior finite membership, and shares sparse Reset handling. A no-change
snapshot preserves exact membership; the actual recovery publisher's regression
failed before this correction and now retains both its anchor and emitted range.
Final independent production review approved the integrated design and diff,
conditional on final gates. Current core library result: 1,818 passed / 11 ignored;
QA binary: 131 passed. Native residual reproduction remained negative after the
bounded additional investigation. One local browser-suite account-lease fixture
race passed all 11 cases on isolated rerun without a product/assertion change;
required CI still gates the full suite at the final PR revision.
