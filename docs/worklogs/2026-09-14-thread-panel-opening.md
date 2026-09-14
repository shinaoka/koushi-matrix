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
