# Thread panel opening after cached pagination

The supplied diagnostic shows repeated thread subscription builds completing,
followed by a zero-millisecond initial page and Opening returning to Closed,
without actor spawn. It cannot distinguish SDK error from premature projection
inspection. Do not copy account logs or message contents into this worklog.

Core inspected a second SDK snapshot immediately after paginate_backwards.
SDK thread pagination sends cache diffs to a separate relay before returning;
its return does not guarantee the display projection is updated. A synthetic
readiness test failed before the fix because it completed with failure before
the delayed projection arrived.

Core now keeps the pre-pagination subscription and applies only its diffs to
its own vector. Successful non-end pagination awaits nonempty content under one
10-second deadline. End-reached remains authoritative, including empty. This
is content readiness, not proof the exact page has fully committed; normal
actor subscription receives subsequent changes. No fixed sleep, retry, SDK
patch, or detached waiter was added. EOF, deadline and SDK failure have distinct
closed diagnostic tokens. The existing SDK-error path remains recoverable.

Four focused async tests pass: delayed publication, end/EOF, non-extending
deadline, and cancellation releasing the stream. Independent review approved
behavior and canon; its missing diagnostic allowlist finding was fixed and
covered by a token test. This unit evidence does not by itself prove the user's
particular failed open had no other cause. Installed-app validation follows.


Build 2738.0 was installed and native accessibility confirmed that clicking the
six-reply summary opened the panel with seven event groups (root plus replies).
Reopening stayed open but exposed only one event group. This revealed a second
independent lifecycle defect: the frontend InitialItems handler replaced actor
generation and items but retained the old actor's EndReached pagination state,
suppressing automatic loading of the replacement actor's partial cache.

A new headless store test failed (expected Idle, actual EndReached). Resetting
both projected pagination directions only on actor-generation changes makes
the 59-test store suite pass; same-actor replay retains its state. Typecheck
passes. Review approved this projection invalidation. Core emits pagination
completion through its existing actor-generation fence, preventing late old
actor completions from overwriting the replacement's state. Core suite after
the opening readiness change: 1,077 passed, 9 ignored.

The earlier unread PR #909 merged at f0344d182a29f636e91f20932fd2838148cb1786;
its monitor has been stopped. This follow-up is on codex/fix-thread-panel-open.


Final build 2739.0 (source `87f6c561`) was signed, installed, and relaunched;
source/installed binary hash and signature checks passed. Full frontend suite:
1,348 passed. Native click verification confirmed the six-reply summary opens
the panel, and close/reopen also leaves the panel open. Room unread remained
zero. The reopened loaded view exposed three latest reply groups, so this
verification does not claim all six replies are simultaneously loaded after
every reopen. The earlier 2738 first-open observation exposed all seven groups.
No raw real-account UI text or diagnostic dump was saved in the repository.

## Follow-up: hidden cached chunks

A later sanitized trace showed initial_backfill_projection_wait followed by
initial_backfill_projection_deadline at 10 seconds, before a subsequent
subscription succeeded. Waiting alone cannot make a hidden-only chunk visible.
The SDK UI Thread paginate_backwards operation uses run_backwards_once, which
can stop after one stored chunk of edits/reactions even when older replies
are already on disk.

`initial_thread_hydrates_across_hidden_cached_chunks` seeds three stored chunks:
root+reply, reaction only, edit only. It builds the real Thread timeline and
calls the production Core hydration helper. Before the correction this fails
the 2-second test deadline despite local visible history. After the correction
it succeeds, projects the reply, and makes no relations/messages requests.

Core uses public ThreadEventCache pagination run_backwards_until with a target
of 100 raw events and holds the cache drop handles. One outer 10-second deadline
covers pagination plus projection readiness. The same subscription is retained
across pagination; typed failure and closed diagnostics remain. The SDK can
consume more than the target when finishing a cached chunk. A window consisting
entirely of hidden events without reaching history end remains subject to the
bounded failure policy; this is not an unlimited history scan.

Validation after both follow-up fixes: Core 1,082 passed / 9 ignored;
frontend 1,348 passed; typecheck passed. Independent design and final-diff
review approved. No SDK changes were needed for these follow-up fixes.
