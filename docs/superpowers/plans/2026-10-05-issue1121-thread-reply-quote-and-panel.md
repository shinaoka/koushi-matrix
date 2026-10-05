# Thread Reply Quote Hydration and Panel Containment (#1121)

**Goal:** Fix the two thread-panel defects tracked by #1121: reply quotes that
render "Unknown user / Original message unavailable" forever (#1120), and the
thread composer identity widening the right panel so its close button is
clipped (#1119).

**Canon:** `docs/architecture/state-machine.md` "Reply quote hydration" owns
the lifecycle; `docs/architecture/overview.md` (message interactions) and
`docs/agents/state-ownership.md` own the layer split. This plan only records
the delivery order.

## Root cause (#1120)

- Thread timelines start with `InitialBackfillPolicy::Disabled`, so the root
  that most thread replies quote is not in the SDK timeline.
- SDK `InReplyToDetails` that were `Unavailable`, `Pending`, or `Error` were
  all projected as terminal `Missing`.
- The only fetch path ran from viewport observation and remembered attempted
  event ids permanently, so a failed or not-yet-observed lookup never retried.
- Pending local echoes were created with no quote at all.

## Delivery

1. Headless reproduction first: the `thread` core scenario records every
   projection of the reply and fails unless the quote lifecycle is
   `Loading* -> Ready` with no `Missing`/`Unsupported`/`Failed`/absent quote
   (`thread_reply_quote_lifecycle=ok`). It failed on main with
   `transaction:none, transaction:missing, event:missing`.
2. Canon amendments: `ReplyQuoteState` gains `Loading` and `Failed`; the
   hydration ledger contract is added to the state machine.
3. `koushi-state`: `ReplyQuoteState::{Loading, Failed}` and wire names.
4. `koushi-core`:
   - `timeline/reply_quote_hydration.rs`: bounded per-actor ledger (256
     entries, 4 in flight, 30 s attempt timeout, retries after 2 s and 10 s
     with `Failed` after the third transient failure; undecryptable originals
     get their own budget, retrying after 15 s, 60 s, 180 s, then 300 s and
     settling `Failed` only after the eighth undecryptable attempt), token
     fencing, the changed-original refresh set, and the known-original overlay
     for canonical batches and pending sends.
   - Lookups use `load_exact_timeline_event_projection` through
     `koushi_core::executor` (`spawn`, `timeout`, `sleep`); tasks are aborted
     on actor drop.
   - Resolved quotes are republished with non-SDK `Set` diffs and pending
     reprojection; the viewport-driven fetch is removed. A republish requested
     while an anchor restore is buffering is deferred until the restore's
     coalesced update has flushed.
   - Pending reply sends start with a `Loading` quote.
5. Frontend: TS mirrors, catalog text for `loading`/`failed` (en + ja), and
   no "unknown sender" row while the original is unresolved.
6. #1119 panel containment lands on the same branch.

## Verification

- Unit tests: `crates/koushi-core/src/timeline/reply_quote_hydration/tests.rs`,
  `crates/koushi-state/tests/message_interactions_state.rs`, and
  `TimelineView.threads.test.tsx`.
- Real server:

```bash
node scripts/desktop-headless-local-qa.mjs --run --server=tuwunel --core --scenario=thread --timeout-ms=500000
```

- Negative control: projecting SDK `Pending` details as `Missing` again makes
  the scenario fail with `thread_reply_quote_lifecycle` violations.
