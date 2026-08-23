# Issue #552 Invite/Mention Query Ownership Leaf

## Recon result

The originally ranked `App.tsx::latestTextOperationQueueRef` cannot be deleted
wholesale without adding replacement Rust semantics:

- staged-upload captions are immediate Rust mutations without request/revision
  admission; concurrent renderer commands could arrive out of order;
- local aliases deliberately reject another request while `Saving`, so the
  frontend serialization ensures the latest typed mutation is eventually sent.

Keep those two mutation workloads serialized. Do not add a generic Rust queue.

Invite target search and mention candidate queries are different: they are
queries, not mutations. Rust already owns their latest-result semantics:

- invite workflow state is destination-scoped and its Rust reducer rebuilds the
  current query projection; #658 fences operation settlement;
- mention targets carry room/surface/query plus monotonically increasing
  generation and request ID; projected/failed results require exact matches;
- Account/Room actors serialize typed command admission and session/refresh
  generations reject stale work;
- `appStore.setAppStoreSnapshot` rejects lower `state_generation` full snapshots
  and delta transport rejects stale/gapped generations.

The shared TS queue currently also suppresses pending invite/mention typed
intents and chooses which promise result may call `setSnapshot`. That duplicates
Rust/appStore query authority. This leaf removes only those query workloads from
the queue while retaining its proven mutation role.

## Change

1. Rename `latestTextOperationQueueRef` and `applyLatestTextSnapshot` to
   mutation-specific names.
2. Keep alias and main/thread caption `run`/`invalidate` behavior unchanged.
3. Remove invite search `run`/invalidate. Send each typed query directly and
   pass every returned snapshot through the existing generation-gated
   `setSnapshot` boundary.
4. Remove main/thread mention `run`. Send each typed query directly; after its
   command settles, fetch/apply a snapshot through the same boundary.
5. Update the #552 inventory row: mutation serialization stays renderer-side
   until a reviewed Rust contract exists; invite/mention latest-result authority
   is migrated to their existing Rust request/generation reducers plus appStore.
   #552 remains open.

No Rust state/action/command, Tauri API, dependency, fake semantic, debounce, or
new abstraction. React still owns text drafts and dispatch timing; it no longer
owns query result admission/coalescing.

## Verify first

Add focused App tests before rewiring:

- delayed invite A/B/A dispatches all three typed queries, settles B/A in an
  adversarial order, and displays only the final Rust projection;
- delayed main and thread mention queries dispatch every typed query; stale
  request/generation projections and lower-generation snapshots cannot replace
  the final room/surface/query target;
- deferred test snapshots must fabricate explicit monotone `state_generation`
  values (the Browser Fake stays at generation 0), following the existing
  Space-members test pattern; otherwise the appStore proof is vacuous;
- account/room/dialog replacement while an old query is pending cannot restore
  the old projection;
- alias and caption A/B/A still serialize and skip superseded pending mutations.

With the current shared queue, the query tests RED behaviorally because an
intermediate pending typed intent is skipped or only the queue-selected promise
may apply. After rewiring, run the unchanged tests GREEN. Use deferred promises
and explicit barriers, never sleeps. Tauri command completion means enqueue, not
actor settlement: tests assert the final rendered projection after explicit
barriers/self-healing refresh, not that each response contains its own command.
The semantic equivalence is final display/state; sending every cheap local query
intent intentionally replaces frontend coalescing.

Also run existing Rust invite-workflow and mention generation tests as ownership
proof, appStore stale/full-snapshot tests, focused App tests, full Vitest,
typecheck/lint/Playwright, formatting/docs/diff.

## Acceptance

| Criterion | Proof |
| --- | --- |
| one semantic owner per migrated query | Rust query/request generations + appStore only |
| TS semantic state removed | no invite/mention key reaches mutation queue |
| delayed/interleaved equivalence | deterministic A/B/A and replacement tests |
| mutation safety preserved | alias/caption queue tests unchanged |
| disjoint Wave C leaf | App tests/docs only; no #659/#608/#559/#570 owners |
| epic remains open | inventory acceptance table updated as partial migration |

Implementation starts only after `reviewer-flash-opencode-go` records
`Correct-to-merge`; exact final diff requires post-review.

## Design review record

- Round 1, `reviewer-flash-opencode-go`: `Correct-to-merge`. Confirmed alias
  `Saving` admission and unversioned caption mutations require retained
  serialization; invite destination scope and mention current-demand/exact
  generation plus appStore monotone generations safely own query convergence.
  Binding execution notes require fabricated nonzero generations and rendered
  final-state assertions because Tauri command completion is enqueue-only.

## Implementation evidence

- RED-first: `npm --prefix apps/desktop test -- --run src/App.inviteMentionOwnership.test.tsx`
  failed before wiring because deferred invite and main/thread mention A/B/A
  dispatch stopped at the first query (`["A"]` / `["a"]`), proving the shared
  queue suppressed intermediate typed intents.
- Focused GREEN: the same test is `6 passed`; the focused App, mutation queue,
  and appStore run is `32 passed` with explicit nonzero snapshot generations,
  adversarial B/old-A/final-A settlement, rendered final projections, and
  dialog/room/account replacement checks.
- The implementation changes only App query dispatch and the TS mutation helper
  naming; alias and main/thread caption mutations retain their queue and
  invalidation paths. No Rust/Tauri API, fake semantics, or dependency changed.
- Initial full gates were GREEN: Vitest 1459/1459, Rust invite/mention ownership
  28/28, browser-headless 260/260, typecheck, lint/agent docs/IME, production
  build, secret scan, SDK submodule and diff checks. One first full browser run
  had an unrelated composer-lease harness flake; its unchanged focused rerun and
  the subsequent complete run both passed.
- After #570 Task C and #608 merged, the leaf rebased onto main
  `be42601d91cf33a46021593d05617857965621d8`; App production auto-merged and the
  plans index retained both branches. Revalidation is GREEN: focused ownership
  and mutation-helper tests 12/12, Vitest 1493/1493, Rust invite/mention filters
  54/54 + 16/16, browser-headless 262/262 with no App unhandled-error signature,
  typecheck, lint/agent docs/IME, production build, SDK submodule and diff checks.
