# #840 first slice: bounded command turns

Status: proposed; implementation requires a recorded design verdict.
This independently mergeable runtime improvement does not complete #840 or
replace its scoped-publication/whole-application migration requirements.

## Current evidence

`AppActor::run` in `crates/koushi-core/src/runtime.rs` processes a first command,
then repeatedly calls `try_recv` until the queue empties. Producers can extend
this turn indefinitely. It also clones `self.state` solely to time a clone;
that `_clone_probe` has no semantic consumer. Normal publication still has real
full-state copies, which remain assigned to the later publication migration.

## Change

- A command turn handles at most 32 commands, including its first command.
  Check the limit before removing another envelope from the channel; leave all
  remaining commands in their original FIFO order. Return to the existing
  select loop after publication and admission settlement. No new queue, task,
  timer, retry, scheduler class or runtime dependency.
- Keep the existing shutdown barrier: when encountered, publish preceding
  mutations and settle their admissions, then stop without processing any
  later command. If shutdown is beyond the turn boundary, encounter it on the
  next turn rather than consuming/discarding it early.
- Continue diffing from the latest published state when a command has its own
  intent commit point. Each turn may emit a delta; global generation ordering
  and per-request admission semantics are unchanged. No promise of one delta
  for the entire pending queue exists; coalescing is per processed batch.
- Delete only the unused diagnostic clone. Report zero clone-probe time for
  the command arm; keep its total elapsed-time diagnostic and the action arm's
  actual pre-state-copy measurement. Do not disguise remaining production
  copies as removed.
- 32 is a work-count bound, not a measured latency guarantee. This change does
  not preempt a slow handler, bound action batches or guarantee select-branch
  fairness. Those remain explicit #840 work; no timeout-based pseudo-guarantee.

## Verify first

Extend the existing runtime test area with one deterministic queue-boundary
regression. On the current-thread runtime, enqueue via non-awaiting try_send
before yielding: 32 commands setting thread order, a 33rd command changing a
different setting, then shutdown. The first settings delta must contain the
first change without the 33rd change; the later delta and final state must
contain both. Baseline coalesces both into its first delta and must fail this
assertion. Count meaningful deltas and preserve ordered shutdown assertions.
Do not depend on wall-clock races or add a general fault-injection framework.

Retain `first_shutdown_publishes_preceding_state_and_ignores_duplicate_and_later_commands`,
command-admission tests and diagnostic threshold tests. Run focused RED→GREEN,
then runtime/Core tests, Rust formatting, applicable boundary/doc checks and
full required PR CI on the final SHA. Build setup is reported separately from
test execution; bound interactive invocations and triage timeouts.

## Canon and acceptance

Amend the existing runtime queue/coalescing contract before implementation to
state the count bound and unchanged ordered shutdown/commit guarantees. Proposed
addition to `docs/architecture/overview.md`, Backpressure rule 11:

> A command batch handles at most 32 commands before returning to the actor's
> event selection. The limit is checked before dequeuing the next envelope;
> FIFO order, intermediate intent commit points and ordered shutdown barriers
> remain intact. This is a command-count bound, not a handler-latency guarantee.

This plan does not authorize changing snapshot/subscription contracts. Final review
must inspect the complete small diff and the RED→GREEN evidence. No source
change is made by this design document.

Design review: GPT-5.6 Sol, read-only, high, 2026-09-06 —
**Correct-to-implement; proposed canon approved**, no blocking findings.
Reviewed the complete plan plus runtime admission/publication/settings/shutdown
paths and the 256-envelope inbox capacity. Optional diagnostic-comment clarity
was applied. This verdict preceded production implementation.

Final full-diff review: DeepSeek V4 Flash, read-only, medium, 2026-09-06 —
**Correct-to-merge subject to required CI**, no blocking findings. Reviewed
patch SHA-256 `6ee3070a01c2145b7aa4af88e5cb3a51c2ceedabcf58f45dae875a3d79ef74ed`.
The optional claim that plain `#[tokio::test]` defaults to multi-thread was
checked against Tokio's macro documentation and rejected: its default test
runtime is current-thread. The test/comment therefore need no change. No
additional review round was requested; only this verdict record was added.

Local evidence: the new regression failed before the loop change with
`RecentFirst` versus expected `Activity` in the first delta (0.09s), then passed
afterward (0.09s). Full Core library tests: **944 passed, 8 ignored**, 6.05s;
existing shutdown/admission and diagnostic tests retained. `cargo fmt --all --
--check` passed. Initial dependency compilation hit its 60s command limit; after
checking for surviving processes, a separate no-run build completed in 54.45s.
This setup timeout was not reported as a test failure or a passing RED check.
Core testkit integration: **225 passed**, no failures or ignored tests. SDK
submodule, domain-dependency, Rust-test-structure and agent-document checks
passed. The isolated PR worktree's two Rust files were byte-compared with the
actual tested files before review; the differing base content is documentation
only. Frontend behavior/dependencies are unchanged; full PR CI remains required.
